use crate::{Error, NodeOptions, NodeRole, material};
mod io;
use meshway_dataplane::{
    DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketBuffer, PacketPool, carrier,
    secure::{
        CircuitAuthorization, Error as SecureError, PendingFrame, Received, SecureNode,
        pipeline::{Job, Output, Pool},
    },
};
use meshway_logging::profile::{self, Metric};
use meshway_model::RequestId;
use meshway_static_authority::public_fixture::PublicFixture;
use meshway_tun::{Options, Tun};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

static TUN_READ: Metric = Metric::new("l3.tun.read");
static UDP_SEND: Metric = Metric::new("l3.udp.send");
static UDP_RECV: Metric = Metric::new("l3.udp.recv");
static TUN_WRITE: Metric = Metric::new("l3.tun.write");
static PACKET: Metric = Metric::new("l3.packet.origin");
static DELIVER: Metric = Metric::new("l3.packet.deliver");
static OWNER: Metric = Metric::new("l3.owner.poll");

fn observe_batch(category: &'static str, count: usize) {
    if count != 0 {
        profile::event(
            category,
            match count {
                1 => "1",
                2..=4 => "2-4",
                5..=8 => "5-8",
                9..=16 => "9-16",
                _ => "17-32",
            },
        );
    }
}

enum Input {
    Send([u8; 32], Packet),
    Receive(meshway_dataplane::secure::Carrier, meshway_transport::Frame),
}

impl Input {
    fn capacity(&self) -> usize {
        match self {
            Self::Send(_, packet) => packet.as_bytes().len() + DEFAULT_HEADROOM + DEFAULT_TAILROOM,
            Self::Receive(_, frame) => frame.payload.capacity(),
        }
    }
}

struct Driver<'a> {
    local: meshway_model::NodeId,
    options: &'a NodeOptions,
    fixture: &'a PublicFixture,
    node: SecureNode,
    pending: VecDeque<(Input, Option<std::time::Instant>)>,
    io_capabilities: serde_json::Value,
    tun: Option<Tun>,
    #[cfg(target_os = "linux")]
    host: Option<meshway_underlay::Host>,
    circuits: BTreeMap<RequestId, [u8; 32]>,
    established: BTreeMap<RequestId, [u8; 32]>,
    ready: bool,
    /// Kernel housekeeping on the TUN (IGMP, ICMPv6 ND/MLD) that no demand
    /// carries; counted and dropped like the daemon's capture boundary.
    unsupported: u64,
    /// Per-packet admission rejections (flow/policy/replay/wire) that the
    /// daemon also drops without stopping the node; authority/state loss stays fatal.
    rejected: u64,
}

/// Rejections scoped to one packet or frame. Everything else ends the run.
fn packet_rejection(error: &SecureError) -> bool {
    matches!(
        error,
        SecureError::Wire
            | SecureError::Packet
            | SecureError::Flow
            | SecureError::Policy
            | SecureError::Proof
            | SecureError::Crypto
            | SecureError::Replay
            | SecureError::Context
    )
}

pub async fn run(
    options: &NodeOptions,
    fixture_path: &Path,
    material_path: &Path,
    stop: CancellationToken,
) -> Result<(), Error> {
    options.validate()?;
    let fixture: PublicFixture = serde_json::from_slice(&std::fs::read(fixture_path)?)?;
    let (local, material) = material::load(material_path)?;
    let index = match options.role {
        NodeRole::A => 0,
        NodeRole::B => fixture.path.len().saturating_sub(1),
        NodeRole::R => 1,
    };
    if fixture.path.get(index) != Some(&local)
        || fixture.sockets.get(&local) != Some(&options.bind)
        || (matches!(options.role, NodeRole::R) && fixture.path.len() != 3)
    {
        return Err("node role/socket differs from public fixture".into());
    }
    let authority = fixture.issue(local, material)?;
    let mut node = SecureNode::new(
        authority.identity(local)?,
        meshway_dataplane::secure::Limits {
            flows: meshway_dataplane::Limits {
                idle_timeout: options.flow_idle,
                ..Default::default()
            },
            ..Default::default()
        },
        BTreeMap::new(),
    )?;
    node.set_carriers(fixture.carriers_for(local)?)?;
    let tun =
        if matches!(options.role, NodeRole::R) || matches!(options.stage, crate::Stage::Manager) {
            None
        } else {
            Some(Tun::create(
                Options::new(options.mtu)?
                    .with_batch_io(true)
                    .with_ipv4(options.role.mesh_address(), 30)?,
            )?)
        };
    let mut io = io::Io::start(
        options,
        &fixture,
        local,
        authority.identity(local)?,
        material_path,
    )
    .await?;
    node.set_carriers(io.carriers(&fixture, local)?)?;
    let mut driver = Driver {
        local,
        options,
        fixture: &fixture,
        node,
        pending: VecDeque::new(),
        io_capabilities: io.capabilities(),
        tun,
        #[cfg(target_os = "linux")]
        host: io.take_host(),
        circuits: BTreeMap::new(),
        established: BTreeMap::new(),
        ready: false,
        unsupported: 0,
        rejected: 0,
    };
    let result = tokio::select! { biased;
        _ = stop.cancelled() => Ok(()),
        result = OWNER.poll(driver.run(&mut io)) => result,
    };
    stop.cancel();
    let capabilities = io.shutdown().await?;
    #[cfg(target_os = "linux")]
    if let Some(host) = driver.host.take() {
        host.shutdown(false).await?;
    }
    eprintln!("{}", serde_json::json!({"ioCapabilities": capabilities}));
    eprintln!(
        "{}",
        serde_json::json!({"event":"secure-node-exit","unsupportedTunPackets":driver.unsupported,"rejectedPackets":driver.rejected})
    );
    // Keep both the issuer and SecureNode alive through every final I/O fence.
    drop(driver);
    drop(authority);
    profile::report_final();
    result
}

impl Driver<'_> {
    fn reject(&mut self, error: SecureError) {
        self.rejected += 1;
        profile::event(
            "l3.reject",
            match error {
                SecureError::Wire => "wire",
                SecureError::Packet => "packet",
                SecureError::Flow => "flow",
                SecureError::Policy => "policy",
                SecureError::Proof => "proof",
                SecureError::Crypto => "crypto",
                SecureError::Replay => "replay",
                SecureError::Context => "context",
                _ => "other",
            },
        );
    }
    fn capture_tun(&self) -> Option<&Tun> {
        #[cfg(target_os = "linux")]
        if let Some(host) = &self.host {
            return host.tun();
        }
        self.tun.as_ref()
    }

    fn tun(&self) -> Option<&Tun> {
        if matches!(self.options.role, NodeRole::R) {
            None
        } else {
            self.capture_tun()
        }
    }
    async fn output(&self, io: &mut io::Io, frame: PendingFrame) -> Result<(), Error> {
        UDP_SEND.poll(io.send(vec![frame], self.fixture)).await?;
        Ok(())
    }

    fn readiness(&mut self) -> Result<(), Error> {
        if !self.ready
            && (self.tun().is_none() || self.established.len() == self.fixture.demands.len())
        {
            self.ready = true;
            println!(
                "{}",
                serde_json::json!({"ready":true, "stage":self.options.stage,
                "role":self.options.role, "tun":self.tun().map(Tun::name),
                "captureTun":self.capture_tun().map(Tun::name),
                "meshAddress":self.tun().map(|_| self.options.role.mesh_address()),
                "mtu":self.options.mtu, "batch":self.options.batch, "profile":profile::enabled(),
                "workers":self.options.workers, "tunBatch":self.capture_tun().map(|tun| tun.capabilities().vnet_header),
                "capabilities":self.io_capabilities,
                "publicationDigest":self.fixture.publication_digest, "circuits":self.established.len()})
            );
            std::io::stdout().flush()?;
        }
        Ok(())
    }

    async fn received(
        &mut self,
        io: &mut io::Io,
        frames: Vec<(meshway_dataplane::secure::Carrier, meshway_transport::Frame)>,
    ) -> Result<(), Error> {
        let mut frames = VecDeque::from(frames);
        while !frames.is_empty() {
            let count = frames.len().min(self.options.batch).min(32);
            self.received_group(io, frames.drain(..count).collect())
                .await?;
        }
        Ok(())
    }

    async fn received_group(
        &mut self,
        io: &mut io::Io,
        frames: Vec<(meshway_dataplane::secure::Carrier, meshway_transport::Frame)>,
    ) -> Result<(), Error> {
        let mut ready = Vec::new();
        observe_batch("l3.udp.read.batch", frames.len());
        let mut at = profile::enabled().then(std::time::Instant::now);
        for (incoming, frame) in frames {
            if frame.payload.get(8) == Some(&3) {
                self.pending
                    .push_back((Input::Receive(incoming, frame), at.take()));
            } else {
                ready.push(self.node.receive(&incoming, frame.payload));
            }
        }
        self.release(io, ready).await
    }

    async fn release(
        &mut self,
        io: &mut io::Io,
        ready: Vec<Result<Received, SecureError>>,
    ) -> Result<(), Error> {
        let mut outgoing = Vec::new();
        let mut deliveries = Vec::new();
        for result in ready {
            let received = match result {
                Ok(received) => received,
                Err(error) if packet_rejection(&error) => {
                    self.reject(error);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            match received {
                Received::Forward(frame) => {
                    if frame.is_data() {
                        outgoing.push(frame);
                    } else {
                        self.output(io, frame).await?;
                    }
                }
                Received::Established { circuit, response } => {
                    let info = self
                        .node
                        .circuit_info(circuit)?
                        .ok_or("missing established circuit")?;
                    let CircuitAuthorization::Business(demand) = info.authorization else {
                        return Err("unexpected diagnostic circuit".into());
                    };
                    self.established.insert(demand, circuit);
                    if let Some(frame) = response {
                        self.output(io, frame).await?;
                    }
                    self.readiness()?;
                }
                Received::Packet(action) => {
                    deliveries.push(action);
                }
                _ => return Err("unexpected diagnostic message".into()),
            }
        }
        if !outgoing.is_empty() {
            UDP_SEND.poll(io.send(outgoing, self.fixture)).await?;
        }
        if !deliveries.is_empty() {
            let count = deliveries.len();
            let tun = self.tun().ok_or("relay attempted plaintext delivery")?;
            TUN_WRITE
                .poll(carrier::deliver_batch(tun, deliveries))
                .await?;
            for _ in 0..count {
                let _packet = DELIVER.enter();
            }
        }
        Ok(())
    }

    fn outbound(&mut self, bytes: PacketBuffer) -> Result<Option<([u8; 32], Packet)>, Error> {
        let Ok(packet) = Packet::parse(bytes) else {
            self.unsupported += 1;
            return Ok(None);
        };
        let protocol = packet.facts().traffic.protocol;
        let Some(demand) = self
            .fixture
            .demands
            .values()
            .find(|demand| demand.protocol == protocol)
        else {
            self.unsupported += 1;
            return Ok(None);
        };
        let circuit = *self
            .established
            .get(&demand.id)
            .ok_or("TUN read before circuit establishment")?;
        Ok(Some((circuit, packet)))
    }

    async fn run(&mut self, io: &mut io::Io) -> Result<(), Error> {
        let mut pool = Pool::new(self.options.workers)?;
        if matches!(self.options.role, NodeRole::A) {
            for demand in self.fixture.demands.keys() {
                let pending = self.node.start(*demand, self.fixture.path.clone())?;
                self.circuits.insert(*demand, pending.circuit());
                self.output(io, pending).await?;
            }
        }
        self.readiness()?;
        let mut report = tokio::time::interval(Duration::from_secs(1));
        let mut retry = tokio::time::interval(Duration::from_millis(100));
        let buffers = PacketPool::new(128, self.options.mtu, DEFAULT_HEADROOM, DEFAULT_TAILROOM)?;
        let mut batch = meshway_tun::PacketBatch::default();
        let mut sends = carrier::SendPending::default();
        let mut deliveries = carrier::TunPending::default();
        loop {
            assert!(self.pending.len() <= 32, "bounded unadmitted input");
            // Match the daemon's scheduling view. Simultaneous authenticated
            // initiators may converge after readiness; the startup carrier map
            // must not pin future operations to the retired dial direction.
            self.node
                .set_carriers(io.carriers(self.fixture, self.local)?)?;
            self.complete_crypto(io, &mut pool, &mut sends, &mut deliveries)?;
            let quantum = std::time::Instant::now();
            let mut prepared = 0;
            for _ in 0..self.pending.len().min(32) {
                if !self
                    .pending
                    .front()
                    .is_some_and(|(input, _)| pool.can_submit_capacity(input.capacity()))
                    || quantum.elapsed() >= Duration::from_micros(200)
                {
                    profile::event(
                        "l3.backpressure",
                        if pool.can_submit() {
                            "quantum"
                        } else {
                            "credit"
                        },
                    );
                    break;
                }
                prepared += 1;
                let (input, at) = self.pending.pop_front().expect("pending prefix");
                if let Some(at) = at {
                    static WALL: Metric = Metric::new("l3.input_head_to_prepare.wall");
                    WALL.elapsed(at.elapsed());
                }
                let job = match input {
                    Input::Send(circuit, packet) => {
                        let _packet = PACKET.enter();
                        self.node
                            .prepare_send_packet(circuit, packet)
                            .map(Job::Send)
                    }
                    Input::Receive(incoming, frame) => self
                        .node
                        .prepare_receive(&incoming, frame.payload.into())
                        .map(Job::Receive),
                };
                match job {
                    Ok(job) => {
                        assert!(pool.submit(job).is_none());
                        if pool.available().is_multiple_of(8) {
                            pool.flush();
                        }
                    }
                    Err(error) if packet_rejection(&error) => self.reject(error),
                    Err(error) => return Err(error.into()),
                }
            }
            observe_batch("l3.prepare.batch", prepared);
            pool.flush();
            self.complete_crypto(io, &mut pool, &mut sends, &mut deliveries)?;
            let can_prepare = self
                .pending
                .front()
                .is_some_and(|(input, _)| pool.can_submit_capacity(input.capacity()));
            let can_read = self.pending.is_empty() && pool.can_submit();
            tokio::select! {
                result = sends.ready(), if !sends.is_empty() => { result?; }
                (result, count) = async {
                    match self.tun() {
                        Some(tun) => deliveries.ready(tun).await,
                        None => std::future::pending().await,
                    }
                }, if !deliveries.is_empty() => {
                    result?;
                    for _ in 0..count { let _packet = DELIVER.enter(); }
                }
                _ = pool.ready(), if pool.has_pending() => {}
                _ = tokio::task::yield_now(), if can_prepare => {}
                received = UDP_RECV.poll(io.recv(can_read)) => {
                    self.received(io, received?).await?;
                }
                read = async {
                    match self.tun() {
                        Some(tun) if self.ready => TUN_READ.poll(tun.recv_batch(&mut batch)).await,
                        _ => std::future::pending().await,
                    }
                }, if can_read => {
                    read?;
                    observe_batch("l3.tun.read.batch", batch.packets().len());
                    let mut at = profile::enabled().then(std::time::Instant::now);
                    for bytes in batch.take() {
                        let mut packet = buffers.acquire(bytes.len())?;
                        packet.copy_from_slice(&bytes);
                        if let Some((circuit, packet)) = self.outbound(packet)? { self.pending.push_back((Input::Send(circuit, packet), at.take())); }
                    }
                }
                _ = retry.tick(), if !self.ready && matches!(self.options.role, NodeRole::A) => {
                    let pending_ids: Vec<_> = self.circuits.iter().filter(|(demand, _)| !self.established.contains_key(demand)).map(|(_, id)| *id).collect();
                    for id in pending_ids {
                        if let Some(frame) = self.node.retry_handshake(id)? { self.output(io, frame).await?; }
                    }
                }
                _ = report.tick() => {
                    #[cfg(feature = "allocator-mimalloc")]
                    if let Ok(stats) = mimalloc::MiMalloc::stats_json() {
                        eprintln!("{}", serde_json::json!({"allocatorStats": stats.to_string_lossy()}));
                    }
                    profile::report();
                    if profile::enabled() {
                        eprintln!("{}", serde_json::json!({"pipelineOccupancy": {
                            "input": self.pending.len(), "creditsAvailable": pool.available(),
                            "tx": sends.occupancy(), "tun": deliveries.occupancy(),
                            "flows": self.node.retained_flows(), "circuits": self.node.retained_circuits(),
                            "tasks": tokio::runtime::Handle::current().metrics().num_alive_tasks(),
                        }}));
                        profile::report_checkpoint();
                    }
                },
            }
            tokio::task::yield_now().await;
        }
    }

    fn complete_crypto(
        &mut self,
        io: &io::Io,
        pool: &mut Pool,
        sends: &mut carrier::SendPending<Result<(), Error>>,
        deliveries: &mut carrier::TunPending<usize>,
    ) -> Result<(), Error> {
        let mut frames = Vec::new();
        let mut packets = Vec::new();
        let mut send_credits = Vec::new();
        let mut delivery_credits = Vec::new();
        for output in pool.take_ready() {
            let (output, credit) = output.into_parts();
            let result = match output {
                Ok(Output::Send(frame)) => Ok(Received::Forward(frame)),
                Ok(Output::Receive(opened)) => self.node.commit_receive(opened),
                Err(error) => Err(error),
            };
            match result {
                Ok(Received::Forward(frame)) => {
                    frames.push(frame);
                    send_credits.push(credit);
                }
                Ok(Received::Packet(packet)) => {
                    packets.push(packet);
                    delivery_credits.push(credit);
                }
                Err(error) if packet_rejection(&error) => self.reject(error),
                Err(error) => return Err(error.into()),
                _ => return Err("unexpected crypto output".into()),
            }
        }
        if !frames.is_empty() {
            let send = io.send(frames, self.fixture);
            sends.push(send_credits, UDP_SEND.poll(send));
        }
        if !packets.is_empty() {
            if self.tun().is_none() {
                return Err("relay attempted plaintext delivery".into());
            }
            let count = packets.len();
            deliveries.push(delivery_credits, packets, count);
        }
        Ok(())
    }
}
