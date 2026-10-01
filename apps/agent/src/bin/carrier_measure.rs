//! Isolated carrier and encrypted Runtime microbenchmarks. Never an agent fallback.
//! VLESS arms use the SAME implementation and handshake; only one removes
//! the already-validated ClientStream wrapper before the measured workload.
use std::{
    collections::VecDeque,
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
#[command(about = "TEST ONLY: matched VLESS adapter and encrypted NativeUDP Runtime measurement")]
struct Cli {
    #[arg(long)]
    uuid_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// TEST ONLY: same encrypted NativeUDP Runtime, synthetic packet I/O, no TUN.
    Runtime {
        #[arg(long)]
        config_file: PathBuf,
        #[arg(long)]
        status_file: PathBuf,
        /// Ordered authorized path; omit to drain received packets at an endpoint/relay.
        #[arg(long, value_delimiter = ',')]
        path: Vec<String>,
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u8).range(1..=30))]
        seconds: u8,
        #[arg(long, default_value_t = 20000, value_parser = clap::value_parser!(u32).range(1000..=64000))]
        packets_per_second: u32,
    },
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

#[derive(Serialize)]
struct ReceiverInterval {
    elapsed_seconds: f64,
    received_packets_total: u64,
    payload_bytes_delta: u64,
    receiver_bits_per_second: f64,
    cgroup_cpu_ns_per_payload_byte: Option<f64>,
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
    if let Command::Runtime {
        config_file,
        status_file,
        path,
        seconds,
        packets_per_second,
    } = cli.command
    {
        return runtime_measure(config_file, status_file, path, seconds, packets_per_second).await;
    }
    let uuid: SecretUuid = fs::read_to_string(
        cli.uuid_file
            .ok_or("--uuid-file required for VLESS measurement")?,
    )?
    .trim()
    .parse()?;
    let target = Target::domain("carrier-measure.invalid", 41000)?;
    match cli.command {
        Command::Runtime { .. } => unreachable!("handled before VLESS credential loading"),
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

async fn runtime_measure(
    config_file: PathBuf,
    status_file: PathBuf,
    path: Vec<String>,
    seconds: u8,
    packets_per_second: u32,
) -> Result<()> {
    #[derive(Deserialize)]
    struct Config {
        dataplane: meshway_dataplane::LabConfig,
    }
    // Reuse Rust-generated lab config and its exact keys/limits/authorization.
    // This measurement deliberately does not instantiate the lab's TUN field.
    let config: Config =
        serde_json::from_str(&meshway_protocol::local::read_private(&config_file)?)?;
    let runtime = meshway_dataplane::Runtime::start(config.dataplane).await?;
    let mut incoming = runtime.take_packet_receiver()?;
    let before = cpu();
    let start = Instant::now();
    let mut received = 0u64;
    let mut sent = 0u64;
    let mut rejected = 0u64;
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    let mut last_bytes = 0u64;
    let mut last_cpu = before;
    let mut last_time = Instant::now();
    let mut intervals = VecDeque::new();
    let publish = |received: u64,
                   sent: u64,
                   rejected: u64,
                   rate: f64,
                   cpu_ns_per_byte: Option<f64>,
                   intervals: &VecDeque<ReceiverInterval>|
     -> Result<()> {
        let report = serde_json::json!({
            "experiment": true, "tun": false, "payload_size": PAYLOAD,
            "node_id": runtime.node_id(), "authenticated_peers": runtime.connected_peers(),
            "statistics": runtime.statistics(), "received_packets": received,
            "accepted_send_packets": sent, "rejected_send_packets": rejected,
            "elapsed_seconds": start.elapsed().as_secs_f64(),
            "last_interval_receiver_bits_per_second": rate,
            "last_interval_cgroup_cpu_ns_per_payload_byte": cpu_ns_per_byte,
            "interval_samples": intervals,
        });
        meshway_protocol::local::atomic_write(
            &status_file,
            &serde_json::to_vec(&report)?,
            status_file.try_exists()?,
        )?;
        Ok(())
    };
    publish(0, 0, 0, 0., None, &intervals)?;
    if path.is_empty() {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                packet = incoming.recv() => {
                    let packet = packet.ok_or("Runtime receiver closed")?;
                    if packet.payload.len() != PAYLOAD || packet.payload[8..].iter().any(|b| *b != 0x5a) {
                        return Err("synthetic payload integrity mismatch".into());
                    }
                    received += 1;
                }
                _ = timer.tick() => {
                    let bytes = received * PAYLOAD as u64;
                    let delta = bytes - last_bytes;
                    let current_cpu = cpu();
                    let cost = last_cpu.zip(current_cpu).filter(|_| delta > 0)
                        .map(|(a,b)| (b-a) as f64 * 1000. / delta as f64);
                    let rate = delta as f64 * 8. / last_time.elapsed().as_secs_f64();
                    if intervals.len() == 32 { intervals.pop_front(); }
                    intervals.push_back(ReceiverInterval {
                        elapsed_seconds: start.elapsed().as_secs_f64(),
                        received_packets_total: received,
                        payload_bytes_delta: delta,
                        receiver_bits_per_second: rate,
                        cgroup_cpu_ns_per_payload_byte: cost,
                    });
                    publish(received, sent, rejected, rate, cost, &intervals)?;
                    last_bytes = bytes; last_cpu = current_cpu; last_time = Instant::now();
                }
            }
        }
    } else {
        runtime
            .wait_for_peer(
                path.get(1).ok_or("path needs next hop")?,
                Duration::from_secs(15),
            )
            .await?;
        // Bounded batches per millisecond. No detached per-packet tasks, and no
        // retransmission layer: loss remains visible instead of hidden by TCP.
        let batch_size = packets_per_second as usize / 1000;
        let mut pace = tokio::time::interval(Duration::from_millis(1));
        pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let load_start = Instant::now();
        while load_start.elapsed() < Duration::from_secs(seconds.into()) {
            pace.tick().await;
            for offset in (0..batch_size).step_by(meshway_dataplane::MAX_BATCH_FRAMES) {
                let count = (batch_size - offset).min(meshway_dataplane::MAX_BATCH_FRAMES);
                let payloads = (0..count)
                    .map(|i| {
                        let mut bytes = vec![0x5a; PAYLOAD];
                        bytes[..8].copy_from_slice(&(sent + rejected + i as u64).to_be_bytes());
                        bytes
                    })
                    .collect();
                match runtime.send_packet_batch(path.clone(), payloads).await {
                    Ok(()) => sent += count as u64,
                    Err(_) => rejected += count as u64,
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        publish(received, sent, rejected, 0., None, &intervals)?;
    }
    runtime.shutdown().await;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let runtime_measurement = matches!(&cli.command, Command::Runtime { .. });
    let mut builder = if runtime_measurement {
        tokio::runtime::Builder::new_multi_thread()
    } else {
        tokio::runtime::Builder::new_current_thread()
    };
    builder.enable_all().build()?.block_on(async {
        let deadline = if runtime_measurement { 600 } else { 60 };
        tokio::time::timeout(Duration::from_secs(deadline), execute(cli)).await?
    })
}
