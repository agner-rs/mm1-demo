use std::collections::VecDeque;
use std::time::{Duration, Instant};

use mm1::address::address::Address;
use mm1::common::log::*;
use mm1::core::context::{dispatch, Ask, InitDone, Quit, Recv, Start, Stop, Tell, Watching};
use mm1::core::prim::AnyError;
use mm1::proto::system;
use mm1::runtime::config::RtConfig;
use mm1::runtime::{Local, Rt};
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
                let mut parts = line.split_ascii_whitespace();
                let Some(cmd) = parts.next() else {
                    eprintln!("no command");
                    continue
                };

                match cmd {
                    "add" => {
                        let Some(delta) = parts.next() else {
                            eprintln!("usage: add <delta: usize>");
                            continue
                        };
                        let Ok(delta) = delta.parse::<usize>() else {
                            eprintln!("couldn't parse delta as usize");
                            continue
                        };

                        info!("adding {} nodes...", delta);
                        for _ in 0..delta {
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
                        info!("nodes.len: {}", nodes.len());
                    },
                    "rm" => {
                        let Some(delta) = parts.next() else {
                            eprintln!("usage: rm <delta: usize>");
                            continue
                        };
                        let Ok(delta) = delta.parse::<usize>() else {
                            eprintln!("couldn't parse delta as usize");
                            continue
                        };

                        info!("removing {} nodes...", delta);
                        for _ in 0..delta {
                            let Some(node) = nodes.pop_back() else {
                                info!("we're out of nodes");
                                break
                            };
                            ctx.shutdown(node, Duration::from_millis(100)).await?;
                        }
                        info!("nodes.len: {}", nodes.len());
                    },
                    "test" => {
                        let Some(c) = parts.next() else {
                            println!("usage: test <C: usize> <N: usize>");
                            continue
                        };
                        let Ok(c) = c.parse::<usize>() else {
                            println!("couldn't parse C as usize");
                            continue
                        };

                        let Some(n) = parts.next() else {
                            println!("usage: test <C: usize> <N: usize>");
                            continue
                        };
                        let Ok(n) = n.parse::<usize>() else {
                            println!("couldn't parse N as usize");
                            continue
                        };

                        if let Some(head) = nodes.front().copied() {
                            let mut forks = vec![];
                            for _ in 0..c {
                                forks.push(ctx.fork().await?);
                            }
                            let mut response_times =
                                futures::future::join_all(forks.into_iter().map(move |mut ctx| {
                                    async move {
                                        let mut response_times = Vec::<Duration>::with_capacity(n);
                                        for _ in 0..n {
                                            let t0 = Instant::now();
                                            if let Err(e) = ctx
                                                .ask(head, |reply_to| {
                                                    protocol::Request { reply_to }
                                                })
                                                .await
                                            {
                                                return Err(e)
                                            }
                                            let dt = t0.elapsed();
                                            response_times.push(dt);
                                        }
                                        Ok(response_times)
                                    }
                                }))
                                .await
                                .into_iter()
                                .collect::<Result<Vec<Vec<Duration>>, _>>()?
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>();

                            // eprintln!("response-times: {:?}", response_times);

                            response_times.sort_unstable();
                            let count = response_times.len();

                            let min = response_times.first();
                            let p50 = response_times.get(count / 2);
                            let p75 = response_times.get(count / 2 + count / 4);
                            let p90 = response_times.get(count - count.div_ceil(10));
                            let p99 = response_times.get(count - count.div_ceil(100));
                            let max = response_times.last();

                            eprintln!("count:\t{}", count);
                            eprintln!("min:\t{:?}", min);
                            eprintln!("p50:\t{:?}", p50);
                            eprintln!("p75:\t{:?}", p75);
                            eprintln!("p90:\t{:?}", p90);
                            eprintln!("p99:\t{:?}", p99);
                            eprintln!("max:\t{:?}", max);
                        } else {
                            warn!("no nodes in the ring");
                        }
                    },
                    unknown => warn!("unknown command: `{}`", unknown),
                }
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
    let _ = mm1_logger::init(&{
        use mm1_logger::*;

        LoggingConfig {
            min_log_level:     Level::INFO,
            log_target_filter: ["mm1_node::*=INFO", "echo_ring=TRACE"]
                .into_iter()
                .map(|s| s.parse())
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
        }
    });
    let rt_config: RtConfig = serde_yaml::from_str(
        r#"
            subnet_address: aaaaaa0000000000/40
            actor_netmask: 52
        "#,
    )
    .expect("parse-config error");
    Rt::create(rt_config)
        .expect("Rt::create")
        .run(Local::actor(main_actor))
        .await
        .expect("Rt::run");
}

mod protocol {
    use mm1::address::address::Address;

    pub struct InputEof;
    pub struct InputLine(pub String);

    pub struct Request {
        pub reply_to: Address,
    }
}
