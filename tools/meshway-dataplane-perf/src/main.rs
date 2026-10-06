#![forbid(unsafe_code)]

use clap::Parser;
use meshway_dataplane_perf::{Error, NodeOptions, NodeRole, Stage, plain};
use std::{net::SocketAddr, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(about = "Isolated L2 plain IP-over-UDP performance node")]
struct Arguments {
    #[arg(long, value_enum)]
    stage: Stage,
    #[arg(long, value_enum)]
    role: NodeRole,
    #[arg(long)]
    bind: SocketAddr,
    #[arg(long)]
    peer: SocketAddr,
    #[arg(long, default_value_t = 1400)]
    mtu: usize,
    #[arg(long, default_value_t = 32)]
    batch: usize,
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
    seconds: u64,
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
    };
    options.validate()?;
    let stop = CancellationToken::new();
    let node = plain::run(&options, stop.clone());
    let shutdown = async {
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = tokio::time::sleep(Duration::from_secs(args.seconds)) => {},
        }
        stop.cancel();
        Ok::<_, Error>(())
    };
    tokio::try_join!(node, shutdown)?;
    Ok(())
}
