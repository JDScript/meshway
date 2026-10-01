//! Isolated, plaintext carrier microbenchmark. Never a Runtime/agent fallback.
//! Both arms use the SAME VLESS implementation and handshake; only one removes
//! the already-validated ClientStream wrapper before the measured workload.
use std::{
    error::Error,
    fs,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand, ValueEnum};
use meshway_transport_vless::{
    ClientConfig, SecretUuid, ServerConfig, Target, client_stream, server_handshake,
};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;
const PAYLOAD: usize = 1280;
const BATCH: usize = 16;
const RECORD: usize = PAYLOAD + 4;

#[derive(Parser)]
#[command(
    about = "TEST ONLY: matched plaintext VLESS adapter measurement; no Meshway identity/E2E/TUN"
)]
struct Cli {
    #[arg(long)]
    uuid_file: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long)]
        listen: SocketAddr,
        #[arg(long)]
        ready_file: PathBuf,
    },
    Run {
        #[arg(long)]
        endpoint: SocketAddr,
        #[arg(long, value_enum)]
        mode: Mode,
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u8).range(1..=10))]
        seconds: u8,
        #[arg(long)]
        reverse: bool,
    },
}

#[derive(Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Direct,
    Wrapped,
}

#[derive(Serialize, Deserialize)]
struct Sample {
    payload_bytes: u64,
    elapsed_ns: u64,
    cgroup_usage_usec: Option<u64>,
}

fn cpu() -> Option<u64> {
    fs::read_to_string("/sys/fs/cgroup/cpu.stat")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("usage_usec ")?.parse().ok())
}

fn sample(bytes: u64, start: Instant, before: Option<u64>) -> Sample {
    Sample {
        payload_bytes: bytes,
        elapsed_ns: start.elapsed().as_nanos() as u64,
        cgroup_usage_usec: before.zip(cpu()).and_then(|(a, b)| b.checked_sub(a)),
    }
}

// Deliberately fixed and identical framing/allocation/batch policy in both arms.
// 16 x (u32-BE length + 1280 bytes), one bounded write per batch, no datagram claim.
async fn send<S: AsyncWrite + Unpin>(stream: &mut S, seconds: u8) -> Result<Sample> {
    let mut batch = [0x5au8; RECORD * BATCH];
    for record in batch.as_chunks_mut::<RECORD>().0 {
        record[..4].copy_from_slice(&(PAYLOAD as u32).to_be_bytes());
    }
    let before = cpu();
    let start = Instant::now();
    let mut bytes = 0;
    while start.elapsed() < Duration::from_secs(seconds.into()) {
        stream.write_all(&batch).await?;
        bytes += (PAYLOAD * BATCH) as u64;
    }
    stream.write_u32(0).await?;
    stream.flush().await?;
    Ok(sample(bytes, start, before))
}

async fn receive<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Sample> {
    let mut payload = [0u8; PAYLOAD];
    let before = cpu();
    let start = Instant::now();
    let mut bytes = 0;
    loop {
        match stream.read_u32().await? as usize {
            0 => break,
            PAYLOAD => {
                stream.read_exact(&mut payload).await?;
                bytes += PAYLOAD as u64;
            }
            _ => return Err("unexpected benchmark record length".into()),
        }
    }
    Ok(sample(bytes, start, before))
}

async fn write_sample<S: AsyncWrite + Unpin>(stream: &mut S, sample: &Sample) -> Result<()> {
    let json = serde_json::to_vec(sample)?;
    stream.write_u32(json.len() as u32).await?;
    stream.write_all(&json).await?;
    Ok(())
}

async fn read_sample<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Sample> {
    let len = stream.read_u32().await? as usize;
    if len > 4096 {
        return Err("oversized benchmark report".into());
    }
    let mut json = vec![0; len];
    stream.read_exact(&mut json).await?;
    Ok(serde_json::from_slice(&json)?)
}

async fn serve(mut socket: TcpStream, config: &ServerConfig) -> Result<()> {
    socket.set_nodelay(true)?;
    server_handshake(&mut socket, config).await?;
    socket.write_u8(0x42).await?; // First read validates the wrapper in BOTH arms.
    loop {
        match socket.read_u8().await? {
            0 => return Ok(()),
            1 => {
                let stats = receive(&mut socket).await?;
                write_sample(&mut socket, &stats).await?;
            }
            2 => {
                let seconds = socket.read_u8().await?;
                if !(1..=10).contains(&seconds) {
                    return Err("invalid duration".into());
                }
                let stats = send(&mut socket, seconds).await?;
                write_sample(&mut socket, &stats).await?;
            }
            3 => {
                let mut payload = [0u8; PAYLOAD];
                socket.read_exact(&mut payload).await?;
                socket.write_all(&payload).await?;
            }
            _ => return Err("invalid benchmark operation".into()),
        }
    }
}

async fn bulk<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    seconds: u8,
    reverse: bool,
) -> Result<(Sample, Sample)> {
    let local = if reverse {
        stream.write_all(&[2, seconds]).await?;
        receive(stream).await?
    } else {
        stream.write_u8(1).await?;
        send(stream, seconds).await?
    };
    let remote = read_sample(stream).await?;
    if local.payload_bytes != remote.payload_bytes || local.payload_bytes == 0 {
        return Err("sender/receiver byte-count mismatch".into());
    }
    Ok(if reverse {
        (remote, local)
    } else {
        (local, remote)
    })
}

#[derive(Serialize)]
struct Report {
    mode: Mode,
    reverse: bool,
    sender: Sample,
    receiver: Sample,
    receiver_bits_per_second: f64,
    total_cgroup_cpu_ns_per_payload_byte: Option<f64>,
    idle_rtt_ns: Vec<u64>,
}

async fn workload<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    mode: Mode,
    seconds: u8,
    reverse: bool,
) -> Result<Report> {
    // One second omitted warmup; its byte count and CPU are not in the report.
    bulk(&mut stream, 1, reverse).await?;
    let (sender, receiver) = bulk(&mut stream, seconds, reverse).await?;
    let mut idle_rtt_ns = Vec::with_capacity(200);
    let mut echo = [0u8; PAYLOAD];
    let request = [0x37u8; PAYLOAD];
    for index in 0..220 {
        let start = Instant::now();
        stream.write_u8(3).await?;
        stream.write_all(&request).await?;
        stream.read_exact(&mut echo).await?;
        if echo != request {
            return Err("echo payload mismatch".into());
        }
        if index >= 20 {
            idle_rtt_ns.push(start.elapsed().as_nanos() as u64);
        }
    }
    stream.write_u8(0).await?;
    let receiver_bits_per_second = receiver.payload_bytes as f64 * 8e9 / receiver.elapsed_ns as f64;
    let total_cgroup_cpu_ns_per_payload_byte = sender
        .cgroup_usage_usec
        .zip(receiver.cgroup_usage_usec)
        .map(|(a, b)| (a + b) as f64 * 1000.0 / receiver.payload_bytes as f64);
    Ok(Report {
        mode,
        reverse,
        sender,
        receiver,
        receiver_bits_per_second,
        total_cgroup_cpu_ns_per_payload_byte,
        idle_rtt_ns,
    })
}

async fn execute(cli: Cli) -> Result<()> {
    let uuid: SecretUuid = fs::read_to_string(cli.uuid_file)?.trim().parse()?;
    let target = Target::domain("carrier-measure.invalid", 41000)?;
    match cli.command {
        Command::Serve { listen, ready_file } => {
            let listener = TcpListener::bind(listen).await?;
            fs::write(ready_file, b"ready")?;
            let (socket, _) = listener.accept().await?;
            serve(
                socket,
                &ServerConfig {
                    uuid,
                    expected_target: target,
                },
            )
            .await?;
        }
        Command::Run {
            endpoint,
            mode,
            seconds,
            reverse,
        } => {
            let socket = TcpStream::connect(endpoint).await?;
            socket.set_nodelay(true)?;
            let mut stream = client_stream(socket, &ClientConfig { uuid, target }).await?;
            if stream.read_u8().await? != 0x42 {
                return Err("missing benchmark greeting".into());
            }
            let report = match mode {
                Mode::Direct => {
                    workload(
                        stream
                            .try_into_inner()
                            .map_err(|_| "unvalidated response")?,
                        mode,
                        seconds,
                        reverse,
                    )
                    .await?
                }
                Mode::Wrapped => workload(stream, mode, seconds, reverse).await?,
            };
            println!("{}", serde_json::to_string(&report)?);
        }
    }
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(60), execute(Cli::parse())).await?
}
