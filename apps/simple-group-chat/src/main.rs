use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use mm1::address::Address;
use mm1::ask::proto::simple::Request;
use mm1::ask::{Ask, Reply};
use mm1::common::error::AnyError;
use mm1::common::log::*;
use mm1::core::context::{Fork, InitDone, Linking, Messaging, Quit, Start, Stop, Tell, Watching};
use mm1::core::envelope::dispatch;
use mm1::proto::sup::uniform;
use mm1::proto::{system, Unique};
use mm1::runtime::{Local, Rt};
use mm1::sup::common::{ChildSpec, ChildType, InitType};
use mm1::sup::uniform::UniformSup;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod protocol {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use mm1::address::Address;
    use mm1::proto::message;

    #[message]
    pub struct Join {
        pub member:    Address,
        pub peer_addr: SocketAddr,
    }

    #[message]
    pub struct Joined {
        pub history: Vec<Arc<[u8]>>,
    }

    #[message]
    pub struct Post {
        pub message: Arc<[u8]>,
    }

    #[message]
    pub struct Message {
        pub message: Arc<[u8]>,
    }
}

#[derive(Debug, thiserror::Error)]
#[error("failure: {}", _0)]
struct Failure(AnyError);

#[derive(Debug, thiserror::Error)]
#[error("eof")]
struct Eof;

async fn room<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone + Messaging + Tell + Watching + Reply,
{
    ctx.init_done(ctx.address()).await;

    let mut members = HashMap::new();
    let mut history = VecDeque::new();

    loop {
        let envelope = ctx.recv().await?;

        dispatch!(match envelope {
            system::Down { peer, .. } => {
                info!("left [member: {}]", peer);
                members.remove(&peer);
            },
            Request::<_> {
                header: reply_to,
                payload: protocol::Join { member, peer_addr },
            } => {
                info!(
                    "join [reply-to: {}; member: {}; peer-addr: {}]",
                    reply_to, member, peer_addr
                );
                let watch_ref = ctx.watch(member).await;
                if let Some(watch_ref) = members.insert(member, watch_ref) {
                    ctx.unwatch(watch_ref).await;
                }
                let history: Vec<_> = history.iter().cloned().collect();
                let _ = ctx.reply(reply_to, protocol::Joined { history }).await;
            },
            Request::<_> {
                header: reply_to,
                payload: protocol::Post { message },
            } => {
                for member in members.keys().copied() {
                    let message = message.clone();
                    let _ = ctx.tell(member, protocol::Message { message }).await;
                }
                history.push_back(message);
                if history.len() > 10 {
                    let _ = history.pop_front();
                }
                let _ = ctx.reply(reply_to, ()).await;
            },
        });
    }
}

async fn acceptor<C>(ctx: &mut C, conn_sup: Address, bind_addr: SocketAddr) -> Result<(), AnyError>
where
    C: Ask + InitDone,
{
    let tcp_listener = TcpListener::bind(bind_addr).await?;
    ctx.init_done(ctx.address()).await;
    loop {
        let (io, _) = tcp_listener.accept().await?;

        let _: uniform::StartResponse = ctx
            .ask(
                conn_sup,
                uniform::StartRequest {
                    args: Unique::new(io),
                },
                Duration::from_secs(3),
            )
            .await?;
    }
}

async fn conn<C>(ctx: &mut C, room: Address, io: Unique<TcpStream>) -> Result<(), AnyError>
where
    C: Messaging + InitDone + Ask + Quit + Fork + Ask,
{
    let io = io.take().expect("stolen IO");
    async fn upstream<C>(
        ctx_up: &mut C,
        room: Address,
        mut io_r: impl AsyncRead + Unpin,
    ) -> Result<(), AnyError>
    where
        C: Ask,
    {
        let mut read_buf = [0u8; 1024];
        loop {
            let byte_count = io_r.read(&mut read_buf).await?;
            if byte_count == 0 {
                break Ok(())
            }
            let message: Arc<[u8]> = read_buf[..byte_count].into();
            ctx_up
                .ask(room, protocol::Post { message }, Duration::from_secs(3))
                .await?;
        }
    }
    async fn downstream<C>(
        ctx_dn: &mut C,
        mut io_w: impl AsyncWrite + Unpin,
    ) -> Result<(), AnyError>
    where
        C: Messaging,
    {
        loop {
            let envelope = ctx_dn.recv().await?;
            let message = dispatch!(match envelope {
                protocol::Message { message } => message,
            });
            io_w.write_all(message.as_ref()).await?;
        }
    }

    ctx.init_done(ctx.address()).await;

    let peer_addr = io.peer_addr()?;

    let ctx_up = ctx.fork().await?;
    let ctx_dn = ctx.fork().await?;
    let (io_r, mut io_w) = io.into_split();

    let downstream_address = ctx_dn.address();

    let protocol::Joined { history } = ctx
        .ask(
            room,
            protocol::Join {
                member: downstream_address,
                peer_addr,
            },
            Duration::from_secs(3),
        )
        .await?;

    info!("joined: [history.len: {}]", history.len());

    for bytes in history {
        io_w.write_all(bytes.as_ref()).await?;
    }

    ctx_up
        .run(move |mut ctx_up| {
            async move {
                if let Err(reason) = upstream(&mut ctx_up, room, io_r).await {
                    warn!("upstream-failed: {}", reason);
                    ctx_up.quit_err(Failure(reason)).await;
                } else {
                    ctx_up.quit_err(Eof).await;
                }
            }
        })
        .await;
    ctx_dn
        .run(move |mut ctx_dn| {
            async move {
                if let Err(reason) = downstream(&mut ctx_dn, io_w).await {
                    warn!("downstream-failed: {}", reason);
                    ctx_dn.quit_err(Failure(reason)).await;
                }
            }
        })
        .await;

    std::future::pending().await
}

async fn conn_sup<C>(ctx: &mut C, room: Address) -> Result<(), AnyError>
where
    C: Quit + InitDone + Start<Local> + Stop + Watching + Linking + Fork + Messaging + Tell,
{
    let launcher = mm1::sup::common::ActorFactoryMut::new(move |tcp_stream| {
        Local::actor((conn, (room, tcp_stream)))
    });
    let child_spec = ChildSpec {
        launcher,
        child_type: ChildType::Temporary,
        init_type: InitType::WithAck {
            start_timeout: Duration::from_millis(100),
        },
        stop_timeout: Duration::from_secs(5),
    };
    let sup_spec = UniformSup::new(child_spec);
    mm1::sup::uniform::uniform_sup(ctx, sup_spec).await?;
    Ok(())
}

async fn main_actor<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local>,
{
    let room = ctx
        .start(Local::actor(room), true, Duration::from_millis(10))
        .await?;
    let conn_sup = ctx
        .start(
            Local::actor((conn_sup, (room,))),
            true,
            Duration::from_millis(10),
        )
        .await?;
    let _acceptor = ctx
        .start(
            Local::actor((acceptor, (conn_sup, "127.0.0.1:8989".parse().unwrap()))),
            true,
            Duration::from_secs(3),
        )
        .await?;

    std::future::pending().await
}

fn main() {
    let _ = mm1_logger::init(&{
        use mm1_logger::*;

        LoggingConfig {
            min_log_level:     Level::INFO,
            log_target_filter: ["mm1_node::*=INFO", "simple_group_chat=DEBUG"]
                .into_iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        }
    });
    Rt::create(Default::default())
        .expect("Rt::create")
        .run(Local::actor(main_actor))
        .expect("Rt::run");
}
