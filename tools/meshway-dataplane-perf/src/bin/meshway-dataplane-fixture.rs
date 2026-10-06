#![forbid(unsafe_code)]
use clap::{Parser, Subcommand};
use meshway_dataplane_perf::{Error, material};
use meshway_static_authority::public_fixture::{PublicFixture, PublicNode};
use std::{net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Parser)]
struct Arguments {
    #[command(subcommand)]
    command: Command,
    #[command(flatten)]
    logging: meshway_logging::Options,
}
#[derive(Subcommand)]
enum Command {
    Material {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        socket: SocketAddr,
    },
    Compile {
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 600)]
        seconds: u64,
    },
}
fn main() -> Result<(), Error> {
    let args = Arguments::parse();
    args.logging.init()?;
    match args.command {
        Command::Material { directory, socket } => println!(
            "{}",
            serde_json::to_string(&material::generate(&directory, socket)?)?
        ),
        Command::Compile { input, seconds } => {
            let nodes: Vec<PublicNode> = serde_json::from_slice(&std::fs::read(input)?)?;
            println!(
                "{}",
                serde_json::to_string(&PublicFixture::iperf(nodes, Duration::from_secs(seconds))?)?
            );
        }
    }
    Ok(())
}
