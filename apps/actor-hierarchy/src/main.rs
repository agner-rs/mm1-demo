use std::io::Read;
use std::time::Duration;

use mm1::common::error::AnyError;
use mm1::common::log::info;
use mm1::core::context::{InitDone, Recv, Start};
use mm1::runtime::{Local, Rt};

fn main() -> Result<(), AnyError> {
    let _ = mm1_logger::init(&{
        use mm1_logger::*;

        LoggingConfig {
            min_log_level:     Level::INFO,
            log_target_filter: ["mm1_node::*=INFO", "actor_hierarchy=TRACE"]
                .into_iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        }
    });

    let mut config = String::new();
    std::io::stdin().read_to_string(&mut config)?;

    let config = serde_yaml::from_str(&config)?;
    Rt::create(config)?.run(Local::actor(main_actor))?;

    Ok(())
}

async fn main_actor<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local>,
{
    info!("main_actor @ {:?}", std::thread::current().name());
    ctx.start(Local::actor(child_1), false, Duration::from_millis(1))
        .await?;
    ctx.start(Local::actor(child_2), false, Duration::from_millis(1))
        .await?;
    Ok(())
}

async fn child_1<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local> + InitDone<Local> + Recv,
{
    info!("1 @ {:?}", std::thread::current().name());

    ctx.start(Local::actor(child_a), false, Duration::from_millis(1))
        .await?;
    ctx.start(Local::actor(child_b), false, Duration::from_millis(1))
        .await?;
    ctx.start(Local::actor(child_c), false, Duration::from_millis(1))
        .await?;

    ctx.init_done(ctx.address()).await;

    Ok(())
}

async fn child_2<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: Start<Local> + InitDone<Local> + Recv,
{
    info!("2 @ {:?}", std::thread::current().name());

    ctx.start(Local::actor(child_a), false, Duration::from_millis(1))
        .await?;
    ctx.start(Local::actor(child_b), false, Duration::from_millis(1))
        .await?;
    ctx.start(Local::actor(child_c), false, Duration::from_millis(1))
        .await?;

    ctx.init_done(ctx.address()).await;

    Ok(())
}

async fn child_a<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone<Local> + Recv,
{
    info!("A @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}

async fn child_b<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone<Local> + Recv,
{
    info!("B @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}

async fn child_c<C>(ctx: &mut C) -> Result<(), AnyError>
where
    C: InitDone<Local> + Recv,
{
    info!("C @ {:?}", std::thread::current().name());
    ctx.init_done(ctx.address()).await;
    Ok(())
}
