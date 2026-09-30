use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use meshway_dataplane::{Runtime, StreamRole, TransportKind};
use meshway_protocol::local;
use serde::{Deserialize, Serialize};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinSet,
    time,
};

use super::{
    Result,
    config::{Config, VlessListener, VlessOutgoing},
};

#[derive(Default)]
struct Counters {
    vless_handshake_failures: AtomicU64,
    tun_sent: AtomicU64,
    tun_received: AtomicU64,
    tun_rejected: AtomicU64,
    tun_send_errors: AtomicU64,
}

struct Session {
    runtime: Arc<Runtime>,
    tasks: JoinSet<Result<()>>,
    counters: Arc<Counters>,
    tun_enabled: bool,
}

impl Session {
    async fn start(config: &Config, enable_tun: bool) -> Result<Self> {
        #[cfg(not(target_os = "linux"))]
        if enable_tun && config.tun.is_some() {
            return Err(io::Error::other("the experimental TUN adapter requires Linux").into());
        }
        let mut listeners = Vec::new();
        for listener in &config.vless_listeners {
            listeners.push((TcpListener::bind(listener.bind).await?, listener.clone()));
        }
        let runtime = Arc::new(Runtime::start(config.dataplane.clone()).await?);
        let counters = Arc::new(Counters::default());
        let mut session = Self {
            runtime,
            tasks: JoinSet::new(),
            counters,
            tun_enabled: false,
        };
        let deadline = Duration::from_millis(config.dataplane.limits.handshake_timeout_ms);
        for (listener, settings) in listeners {
            session.tasks.spawn(accept_vless(
                session.runtime.clone(),
                listener,
                settings,
                deadline,
                config.dataplane.limits.max_connections,
                session.counters.clone(),
            ));
        }
        for outgoing in &config.vless_outgoing {
            session.tasks.spawn(connect_vless(
                session.runtime.clone(),
                outgoing.clone(),
                deadline,
                Duration::from_millis(config.dataplane.limits.reconnect_delay_ms),
                session.counters.clone(),
            ));
        }
        #[cfg(target_os = "linux")]
        if let Some(settings) = config.tun.as_ref().filter(|_| enable_tun) {
            let device = match super::tun::open(settings) {
                Ok(device) => Arc::new(device),
                Err(error) => {
                    session.stop().await;
                    return Err(error.into());
                }
            };
            session.tun_enabled = true;
            let received = session.runtime.take_packet_receiver()?;
            session.tasks.spawn(run_tun(
                session.runtime.clone(),
                device,
                settings.clone(),
                received,
                session.counters.clone(),
            ));
        }
        Ok(session)
    }

    async fn stop(&mut self) {
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        self.runtime.shutdown().await;
    }
}

async fn accept_vless(
    runtime: Arc<Runtime>,
    listener: TcpListener,
    settings: VlessListener,
    deadline: Duration,
    limit: usize,
    counters: Arc<Counters>,
) -> Result<()> {
    let adapter = settings.adapter()?;
    let mut pending = JoinSet::new();
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (mut stream, _) = result?;
                if pending.len() >= limit {
                    counters.vless_handshake_failures.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                stream.set_nodelay(true)?;
                let runtime = runtime.clone();
                let adapter = adapter.clone();
                let counters = counters.clone();
                pending.spawn(async move {
                    let result = time::timeout(deadline, async {
                        meshway_transport_vless::server_handshake(&mut stream, &adapter).await?;
                        runtime.attach_stream(stream, StreamRole::Acceptor, TransportKind::Vless).await?;
                        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
                    }).await;
                    if !matches!(result, Ok(Ok(()))) {
                        counters.vless_handshake_failures.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
            result = pending.join_next(), if !pending.is_empty() => {
                if let Some(Err(error)) = result {
                    return Err(io::Error::other(format!("VLESS handshake worker failed: {error}")).into());
                }
            }
        }
    }
}

async fn connect_vless(
    runtime: Arc<Runtime>,
    settings: VlessOutgoing,
    deadline: Duration,
    reconnect: Duration,
    counters: Arc<Counters>,
) -> Result<()> {
    let adapter = settings.adapter()?;
    loop {
        // This experiment uses one externally managed VLESS candidate per peer.
        // Keep the accepted channel alive instead of continuously redialing it.
        if runtime.connected_peers().contains(&settings.peer) {
            time::sleep(reconnect).await;
            continue;
        }
        let result = time::timeout(deadline, async {
            let stream = TcpStream::connect(settings.endpoint).await?;
            stream.set_nodelay(true)?;
            // External proxies may wait for client-first Noise before emitting
            // the VLESS response. Lazy header consumption avoids that deadlock.
            let stream = meshway_transport_vless::client_stream(stream, &adapter).await?;
            runtime
                .attach_stream_with_priority(
                    stream,
                    StreamRole::Dialer {
                        peer: settings.peer.clone(),
                    },
                    TransportKind::Vless,
                    settings.priority,
                )
                .await?;
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        })
        .await;
        if !matches!(result, Ok(Ok(()))) {
            counters
                .vless_handshake_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        time::sleep(reconnect).await;
    }
}

#[cfg(target_os = "linux")]
async fn run_tun(
    runtime: Arc<Runtime>,
    device: Arc<tun_rs::AsyncDevice>,
    settings: super::tun::TunConfig,
    mut incoming: tokio::sync::mpsc::Receiver<meshway_dataplane::ReceivedPacket>,
    counters: Arc<Counters>,
) -> Result<()> {
    let outbound = async {
        let mut buffer = vec![0; 65536];
        loop {
            let size = device.recv(&mut buffer).await?;
            let mut packets = Vec::with_capacity(meshway_dataplane::MAX_BATCH_FRAMES);
            if settings.accepts_outgoing(&buffer[..size]) {
                packets.push(buffer[..size].to_vec());
            } else {
                counters.tun_rejected.fetch_add(1, Ordering::Relaxed);
            }
            // Drain only a bounded batch; a busy device must not starve control.
            for _ in 1..meshway_dataplane::MAX_BATCH_FRAMES {
                match device.try_recv(&mut buffer) {
                    Ok(size) if settings.accepts_outgoing(&buffer[..size]) => {
                        packets.push(buffer[..size].to_vec())
                    }
                    Ok(_) => {
                        counters.tun_rejected.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => {
                        return Err::<(), Box<dyn std::error::Error + Send + Sync>>(error.into());
                    }
                }
            }
            if packets.is_empty() {
                continue;
            }
            let count = packets.len() as u64;
            match runtime
                .send_packet_batch(settings.send_path.clone(), packets)
                .await
            {
                Ok(()) => {
                    counters.tun_sent.fetch_add(count, Ordering::Relaxed);
                }
                Err(_) => {
                    counters.tun_send_errors.fetch_add(count, Ordering::Relaxed);
                }
            }
        }
    };
    let inbound = async {
        while let Some(packet) = incoming.recv().await {
            if settings.receive_path.first() != Some(&packet.source)
                || !settings.accepts_incoming(&packet.path, &packet.payload)
            {
                counters.tun_rejected.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            let written = device.send(&packet.payload).await?;
            if written != packet.payload.len() {
                return Err::<(), Box<dyn std::error::Error + Send + Sync>>(
                    io::Error::other("partial TUN packet write").into(),
                );
            }
            counters.tun_received.fetch_add(1, Ordering::Relaxed);
        }
        Err(io::Error::other("dataplane packet receiver closed").into())
    };
    tokio::try_join!(outbound, inbound)?;
    Ok(())
}

#[derive(Serialize)]
struct TunStatistics {
    sent: u64,
    received: u64,
    rejected: u64,
    send_errors: u64,
}

async fn publish_status(session: &Session, config: &Config, path: &std::path::Path) -> Result<()> {
    let authenticated_peers = session.runtime.connected_peers();
    let counters = &session.counters;
    let status = serde_json::json!({
        "experiment": true,
        "observed_at": chrono::Utc::now(),
        "node_id": config.dataplane.node_id,
        "authenticated_peers": authenticated_peers,
        "tun_enabled": session.tun_enabled,
        "statistics": session.runtime.statistics(),
        "vless_handshake_failures": counters.vless_handshake_failures.load(Ordering::Relaxed),
        "tun": TunStatistics {
            sent: counters.tun_sent.load(Ordering::Relaxed),
            received: counters.tun_received.load(Ordering::Relaxed),
            rejected: counters.tun_rejected.load(Ordering::Relaxed),
            send_errors: counters.tun_send_errors.load(Ordering::Relaxed),
        },
    });
    local::atomic_write(path, &serde_json::to_vec(&status)?, path.try_exists()?)?;
    Ok(())
}

pub async fn run(config: Config, status_file: Option<PathBuf>) -> Result<()> {
    let _status_lock = if let Some(path) = &status_file {
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        local::private_dir(parent)?;
        let lock = local::lock(&path.with_extension("status-lock"))?;
        if path.try_exists()? {
            #[derive(Deserialize)]
            struct ExistingStatus {
                experiment: bool,
                node_id: String,
            }
            let previous: ExistingStatus = serde_json::from_str(&local::read_private(path)?)
                .map_err(|_| {
                    io::Error::other("status destination already contains another kind of file")
                })?;
            if !previous.experiment || previous.node_id != config.dataplane.node_id {
                return Err(io::Error::other(
                    "status destination belongs to a different experiment node",
                )
                .into());
            }
        }
        Some(lock)
    } else {
        None
    };
    let mut session = Session::start(&config, true).await?;
    println!(
        "Experimental carriers started for {}; authenticated peer readiness is reported separately.",
        config.dataplane.node_id
    );
    let mut interval = time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => break signal.map_err(Into::into),
            result = session.tasks.join_next(), if !session.tasks.is_empty() => {
                break match result {
                    Some(Ok(Err(error))) => Err(error),
                    _ => Err(io::Error::other("experimental carrier task stopped unexpectedly").into()),
                };
            }
            _ = interval.tick(), if status_file.is_some() => {
                if let Some(path) = &status_file
                    && let Err(error) = publish_status(&session, &config, path).await {
                    break Err(error);
                }
            }
        }
    };
    session.stop().await;
    result
}

pub async fn probe(config: Config, path: Vec<String>, size: usize, count: u16) -> Result<()> {
    let mut session = Session::start(&config, false).await?;
    let result = async {
        let next = path.get(1).ok_or_else(|| io::Error::other("probe needs a next hop"))?;
        session.runtime.wait_for_peer(next, Duration::from_secs(10)).await?;
        let mut measurements = Vec::with_capacity(usize::from(count));
        for round in 0..count {
            let payload: Vec<u8> = (0..size).map(|index| (index as u8).wrapping_add(round as u8)).collect();
            let started = time::Instant::now();
            let reply = session.runtime.probe(path.clone(), payload.clone()).await?;
            if reply.payload != payload {
                return Err::<(), Box<dyn std::error::Error + Send + Sync>>(io::Error::other("endpoint-authenticated echo payload mismatch").into());
            }
            measurements.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{}", serde_json::to_string(&serde_json::json!({
            "experiment": true, "kind": "endpoint_authenticated_echo", "authorized_path": path,
            "bytes_per_echo": size, "round_trip_ms": measurements,
            "note": "Path is authorized intent; this is not measured per-hop trace or iperf throughput.",
        }))?);
        Ok(())
    }.await;
    session.stop().await;
    result
}
