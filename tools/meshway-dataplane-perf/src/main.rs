#![forbid(unsafe_code)]

use clap::Parser;
use meshway_dataplane_perf::{Error, NodeOptions, NodeRole, Stage, plain, secure};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio_util::sync::CancellationToken;

// Controlled allocator comparison belongs only to this measurement executable.
#[cfg(feature = "allocator-mimalloc")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(about = "Isolated L2/L3/L4 TUN-over-UDP performance node")]
struct Arguments {
    #[arg(long, value_enum)]
    stage: Stage,
    #[arg(long, value_enum)]
    role: NodeRole,
    #[arg(long)]
    bind: SocketAddr,
    #[arg(long)]
    peer: SocketAddr,
    #[arg(long)]
    fixture: Option<PathBuf>,
    #[arg(long)]
    material: Option<PathBuf>,
    #[arg(long, default_value_t = 1400)]
    mtu: usize,
    #[arg(long, default_value_t = 32)]
    batch: usize,
    #[arg(long, default_value_t = meshway_dataplane::secure::pipeline::Pool::default_workers())]
    workers: usize,
    #[arg(long, value_enum, default_value_t = plain::IoMode::PerPacket)]
    io: plain::IoMode,
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
    seconds: u64,
    #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=86400))]
    flow_idle_seconds: u64,
    #[command(flatten)]
    logging: meshway_logging::Options,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let args = Arguments::parse();
    args.logging.init()?;
    let options = NodeOptions {
        stage: args.stage,
        role: args.role,
        bind: args.bind,
        peer: args.peer,
        mtu: args.mtu,
        batch: args.batch,
        workers: args.workers,
        flow_idle: Duration::from_secs(args.flow_idle_seconds),
    };
    options.validate()?;
    let stop = CancellationToken::new();
    let node = async {
        match options.stage {
            Stage::Plain => plain::run_with_io(&options, stop.clone(), args.io).await,
            Stage::Secure | Stage::Manager => {
                secure::run(
                    &options,
                    args.fixture
                        .as_deref()
                        .ok_or("secure stage requires --fixture")?,
                    args.material
                        .as_deref()
                        .ok_or("secure stage requires --material")?,
                    stop.clone(),
                )
                .await
            }
        }
    };
    let shutdown = async {
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = tokio::time::sleep(Duration::from_secs(args.seconds)) => {},
        }
        stop.cancel();
        Ok::<_, Error>(())
    };
    let execution = async {
        tokio::try_join!(node, shutdown)?;
        Ok::<_, Error>(())
    };
    tokio::select! {
        result = execution => result?,
        result = checkpoints() => result?,
    }
    Ok(())
}

async fn checkpoints() -> Result<(), Error> {
    #[cfg(unix)]
    {
        let mut signal =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())?;
        while signal.recv().await.is_some() {
            meshway_logging::profile::report_checkpoint();
        }
    }
    std::future::pending().await
}
