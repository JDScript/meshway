#![forbid(unsafe_code)]
use clap::{Parser, Subcommand};
use meshway_dataplane_perf::{Error, material};
use meshway_desired_fixture::public_fixture::{PublicFixture, PublicNode};
use std::{net::SocketAddr, path::PathBuf};

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
        Command::Compile { input } => {
            let nodes: Vec<PublicNode> = serde_json::from_slice(&std::fs::read(input)?)?;
            println!("{}", serde_json::to_string(&PublicFixture::iperf(nodes)?)?);
        }
    }
    Ok(())
}
