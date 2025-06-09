use std::io::Read;
use std::time::Duration;

use mm1::common::error::AnyError;
use mm1::common::log::info;
use mm1::core::context::{InitDone, Messaging, Start, Tell};
use mm1::message_codec::Codec;
use mm1::proto::message;
use mm1::runnable;
use mm1::runtime::{Local, Rt};

fn main() -> Result<(), AnyError> {
    let _ = mm1_logger::init(&{
        use mm1_logger::*;

        LoggingConfig {
            min_log_level:     Level::TRACE,
            log_target_filter: [
                "mm1_node::*=DEBUG",
                "mm1_multinode::*=DEBUG",
                "actor_hierarchy=TRACE",
            ]
            .into_iter()
            .map(|s| s.parse())
            .collect::<Result<Vec<_>, _>>()
            .unwrap(),
        }
    });

    let mut config = String::new();
    std::io::stdin().read_to_string(&mut config)?;

    let config = serde_yaml::from_str(&config)?;
    Rt::create(config)?
        .with_codec("full", Codec::new().with_type::<AMessage>())
        .run(runnable::local::boxed_from_fn(main_actor))?;

    Ok(())
}

async fn main_actor<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local> + Messaging,
{
    info!("main_actor @ {:?}", std::thread::current().name());
    ctx.start(
        runnable::local::boxed_from_fn(child_1),
        false,
        Duration::from_millis(1),
    )
    .await?;
    ctx.start(
        runnable::local::boxed_from_fn(child_2),
        false,
        Duration::from_millis(1),
    )
    .await?;

    ctx.tell("<bad:13>".parse().unwrap(), AMessage).await?;

    std::future::pending().await
}

async fn child_1<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local> + InitDone + Messaging,
{
    info!("1 @ {:?}", std::thread::current().name());

    ctx.start(
        runnable::local::boxed_from_fn(child_a),
        false,
        Duration::from_millis(1),
    )
    .await?;
    ctx.start(
        runnable::local::boxed_from_fn(child_b),
        false,
        Duration::from_millis(1),
    )
    .await?;
    ctx.start(
        runnable::local::boxed_from_fn(child_c),
        false,
        Duration::from_millis(1),
    )
    .await?;

    ctx.init_done(ctx.address()).await;

    Ok(())
}

async fn child_2<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local> + InitDone + Messaging,
{
    info!("2 @ {:?}", std::thread::current().name());

    ctx.start(
        runnable::local::boxed_from_fn(child_a),
        false,
        Duration::from_millis(1),
    )
    .await?;
    ctx.start(
        runnable::local::boxed_from_fn(child_b),
        false,
        Duration::from_millis(1),
    )
    .await?;
    ctx.start(
        runnable::local::boxed_from_fn(child_c),
        false,
        Duration::from_millis(1),
    )
    .await?;

    ctx.init_done(ctx.address()).await;

    Ok(())
}

async fn child_a<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone + Messaging,
{
    info!("A @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}

async fn child_b<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone + Messaging,
{
    info!("B @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}

async fn child_c<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone + Messaging,
{
    info!("C @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}

#[message]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct AMessage;
