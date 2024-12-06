use std::collections::VecDeque;
use std::time::{Duration, Instant};

use kendall::address::address::Address;
use kendall::common::log::*;
use kendall::core::context::{dispatch, Ask, InitDone, Quit, Recv, Start, Stop, Tell, Watching};
use kendall::core::prim::AnyError;
use kendall::proto::system;
use kendall::runtime::{Local, Rt};
use tokio::io::{AsyncBufReadExt, BufReader};

async fn main_actor<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Quit + Recv + Tell + Ask + Start<Local> + Stop<Local> + Watching<Local>,
{
    let mut nodes = VecDeque::<Address>::new();

    ctx.fork()
        .await?
        .run({
            let address = ctx.address();
            let mut lines = BufReader::new(tokio::io::stdin()).lines();
            move |mut ctx| {
                async move {
                    loop {
                        match lines.next_line().await {
                            Ok(None) => {
                                let _ = ctx.tell(address, protocol::InputEof).await;
                            },
                            Ok(Some(line)) => {
                                let _ = ctx.tell(address, protocol::InputLine(line)).await;
                            },
                            Err(reason) => {
                                warn!("stdin error: {}", reason);
                                ctx.quit_err(reason).await;
                            },
                        }
                    }
                }
            }
        })
        .await;

    loop {
        dispatch!(match ctx.recv().await? {
            protocol::InputEof => {
                info!("Bye!");
                ctx.quit_ok().await;
            },
            protocol::InputLine(line) => {
                let Ok(delta) = line.parse::<isize>() else {
                    warn!("couldn't parse input as isize");
                    continue
                };

                let current = nodes.len();
                let Some(target) = current.checked_add_signed(delta) else {
                    warn!(
                        "target count is out of bounds [current: {}; delta: {}]",
                        current, delta
                    );
                    continue;
                };

                let delta_abs = delta.abs();

                use std::cmp::Ordering::*;
                match delta.cmp(&0) {
                    Less => {
                        info!("removing {} nodes...", delta_abs);
                        for _ in 0..delta_abs {
                            let node = nodes.pop_back().expect("shouldn't we have enough nodes?");
                            ctx.shutdown(node, Duration::from_millis(100)).await?;
                        }
                    },
                    Greater => {
                        info!("adding {} nodes...", delta_abs);

                        for _ in 0..delta_abs {
                            let relay_to = nodes.front().copied();
                            let node = ctx
                                .start(
                                    Local::actor((node, (relay_to,))),
                                    false,
                                    Duration::from_millis(1),
                                )
                                .await?;
                            nodes.push_front(node);
                        }
                    },
                    Equal => {
                        if let Some(head) = nodes.front().copied() {
                            info!("request-start");
                            let t0 = Instant::now();
                            let _ = ctx
                                .ask(head, |reply_to| protocol::Request { reply_to })
                                .await?;
                            let dt = t0.elapsed();
                            info!("request-done [dt: {:?}]", dt);
                        } else {
                            warn!("no nodes in the ring");
                        }
                    },
                }

                info!("nodes.len: {}", nodes.len());
                assert_eq!(target, nodes.len());
            },
        })
    }
}

async fn node<C>(ctx: &mut C, relay_to: Option<Address>) -> Result<(), AnyError>
where
    C: InitDone<Local> + Recv + Tell + Watching<Local>,
{
    ctx.init_done(ctx.address()).await;

    if let Some(relay_to) = relay_to {
        let w = ctx.watch(relay_to).await;

        loop {
            dispatch!(match ctx.recv().await? {
                system::Down { watch_ref, .. } if *watch_ref == w => {
                    break
                },
                req @ protocol::Request { .. } => {
                    let _ = ctx.tell(relay_to, req).await;
                },
            })
        }
    }

    loop {
        dispatch!(match ctx.recv().await? {
            protocol::Request { reply_to } => {
                let _ = ctx.tell(reply_to, ()).await;
            },
        })
    }
}

#[tokio::main]
async fn main() {
    let _ = kendall_logger::init(&{
        use kendall_logger::*;

        LoggingConfig {
            min_log_level:     Level::INFO,
            log_target_filter: ["kendall_node::*=INFO", "echo_ring=TRACE"]
                .into_iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        }
    });
    Rt::create(Default::default())
        .expect("Rt::create")
        .run(Local::actor(main_actor))
        .await
        .expect("Rt::run");
}

mod protocol {
    use kendall::address::address::Address;

    pub struct InputEof;
    pub struct InputLine(pub String);

    pub struct Request {
        pub reply_to: Address,
    }
}
