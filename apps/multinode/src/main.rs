use std::time::Duration;

use futures::never::Never;
use mm1::address::Address;
use mm1::common::error::AnyError;
use mm1::common::log;
use mm1::common::log::info;
use mm1::core::context::{Messaging, Tell};
use mm1::core::envelope::dispatch;
use mm1::message_codec::Codec;
use mm1::proto::message;
use mm1::runnable;
use mm1::runtime::Rt;
use tokio::sync::oneshot;
use tokio::time;

fn main() -> Result<(), AnyError> {
    let _ = mm1_logger::init(&{
        use mm1_logger::*;

        LoggingConfig {
            min_log_level:     Level::TRACE,
            log_target_filter: [
                "mm1_node::*=DEBUG",
                "mm1_multinode::*=DEBUG",
                "multinode=TRACE",
            ]
            .into_iter()
            .map(|s| s.parse())
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        }
    });

    let (tx, rx) = oneshot::channel();

    let a = std::thread::spawn(move || run_a(tx));
    let b = std::thread::spawn(move || run_b(rx));

    a.join().expect("ew... a")?;
    b.join().expect("ew... b")?;

    Ok(())
}

fn run_a(tx: oneshot::Sender<Address>) -> Result<(), AnyError> {
    let config = serde_yaml::from_str(
        r#"
        codecs:
            - name: full
        subnets:
            - net_address: <a:>/16
              type: local
            - net_address: <b:>/16
              type: remote
              codec: full
              protocol: wip
              link:
                bind: 127.0.0.1:12001
                peer: 127.0.0.1:12002
        "#,
    )?;
    Rt::create(config)?
        .with_codec("full", Codec::new().with_type::<AMessage>())
        .run(runnable::local::boxed_from_fn((main_a_actor, (tx,))))?;

    Ok(())
}

fn run_b(rx: oneshot::Receiver<Address>) -> Result<(), AnyError> {
    let config = serde_yaml::from_str(
        r#"
        codecs:
            - name: full
        subnets:
            - net_address: <b:>/16
              type: local
            - net_address: <a:>/16
              type: remote
              codec: full
              protocol: wip
              link:
                bind: 127.0.0.1:12002
                peer: 127.0.0.1:12001
        "#,
    )?;
    Rt::create(config)?
        .with_codec("full", Codec::new().with_type::<AMessage>())
        .run(runnable::local::boxed_from_fn((main_b_actor, (rx,))))?;

    Ok(())
}

async fn main_a_actor<C>(ctx: &mut C, tx: oneshot::Sender<Address>) -> Result<(), AnyError>
where
    C: Messaging,
{
    info!("A @ {:?}", std::thread::current().name());
    tx.send(ctx.address()).expect("ew... tx.send");

    match main_loop(ctx).await? {}
}

async fn main_b_actor<C>(ctx: &mut C, rx: oneshot::Receiver<Address>) -> Result<(), AnyError>
where
    C: Messaging,
{
    info!("B @ {:?}", std::thread::current().name());
    let peer = rx.await.expect("ew... rx.await");

    ctx.tell(
        peer,
        AMessage {
            reply_to: ctx.address(),
            idx:      1,
        },
    )
    .await?;

    match main_loop(ctx).await? {}
}

async fn main_loop<Ctx>(ctx: &mut Ctx) -> Result<Never, AnyError>
where
    Ctx: Messaging,
{
    loop {
        let envelope = ctx.recv().await?;
        log::info!("received {:?}", envelope);

        let AMessage { reply_to, idx } = dispatch!(match envelope {
            m @ AMessage { .. } => m,
        });

        time::sleep(Duration::from_secs(1)).await;

        ctx.tell(
            reply_to,
            AMessage {
                reply_to: ctx.address(),
                idx:      idx + 1,
            },
        )
        .await?;
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[message]
struct AMessage {
    reply_to: Address,
    idx:      usize,
}
