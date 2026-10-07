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

struct Driver<'a> {
    options: &'a NodeOptions,
    fixture: &'a PublicFixture,
    node: SecureNode,
    pool: Pool,
    io_capabilities: serde_json::Value,
    tun: Option<Tun>,
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
        Default::default(),
        BTreeMap::new(),
    )?;
    node.set_carriers(fixture.carriers_for(local)?)?;
    let tun = if matches!(options.role, NodeRole::R) {
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
        options,
        fixture: &fixture,
        node,
        pool: Pool::new(options.workers)?,
        io_capabilities: io.capabilities(),
        tun,
        circuits: BTreeMap::new(),
        established: BTreeMap::new(),
        ready: false,
        unsupported: 0,
        rejected: 0,
    };
    let result = tokio::select! { biased;
        _ = stop.cancelled() => Ok(()),
        result = driver.run(&mut io) => result,
    };
    stop.cancel();
    let capabilities = io.shutdown().await?;
    eprintln!("{}", serde_json::json!({"ioCapabilities": capabilities}));
    eprintln!(
        "{}",
        serde_json::json!({"event":"secure-node-exit","unsupportedTunPackets":driver.unsupported,"rejectedPackets":driver.rejected})
    );
    // Keep both the issuer and SecureNode alive through every final I/O fence.
    drop(driver);
    drop(authority);
    // report() is rate-limited by the shared logging owner. A quiet final
    // interval yields one complete cumulative snapshot after all I/O has joined.
    if profile::enabled() {
        tokio::time::sleep(Duration::from_millis(5010)).await;
    }
    profile::report();
    result
}

impl Driver<'_> {
    async fn output(&self, io: &mut io::Io, frame: PendingFrame) -> Result<(), Error> {
        UDP_SEND.poll(io.send(vec![frame], self.fixture)).await?;
        Ok(())
    }

    fn readiness(&mut self) -> Result<(), Error> {
        if !self.ready
            && (self.tun.is_none() || self.established.len() == self.fixture.demands.len())
        {
            self.ready = true;
            println!(
                "{}",
                serde_json::json!({"ready":true, "stage":self.options.stage,
                "role":self.options.role, "tun":self.tun.as_ref().map(Tun::name),
                "meshAddress":self.tun.as_ref().map(|_| self.options.role.mesh_address()),
                "mtu":self.options.mtu, "batch":self.options.batch, "profile":profile::enabled(),
                "workers":self.pool.workers(), "tunBatch":self.tun.as_ref().map(|tun| tun.capabilities().vnet_header),
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
        let mut jobs = VecDeque::new();
        let mut ready = Vec::new();
        for (incoming, frame) in frames {
            if frame.payload.get(8) == Some(&3) {
                match self.node.prepare_receive(&incoming, frame.payload.into()) {
                    Ok(job) => {
                        jobs.push_back(Job::Receive(job));
                        ready.push(None);
                    }
                    Err(error) => ready.push(Some(Err(error))),
                }
            } else {
                ready.push(Some(self.node.receive(&incoming, frame.payload)));
            }
        }
        let mut opened = self.pool.run(&mut jobs).await.into_iter();
        let mut outgoing = Vec::new();
        let mut deliveries = Vec::new();
        for result in ready {
            let result =
                result.unwrap_or_else(|| match opened.next().expect("one completion per job") {
                    Ok(Output::Receive(opened)) => self.node.commit_receive(opened),
                    Ok(Output::Send(_)) => Err(SecureError::State),
                    Err(error) => Err(error),
                });
            let received = match result {
                Ok(received) => received,
                Err(error) if packet_rejection(&error) => {
                    self.rejected += 1;
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
            let tun = self
                .tun
                .as_ref()
                .ok_or("relay attempted plaintext delivery")?;
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
        if matches!(self.options.role, NodeRole::A) {
            for demand in self.fixture.demands.keys() {
                let pending = self.node.start(*demand, self.fixture.path.clone())?;
                self.circuits.insert(*demand, pending.circuit());
                self.output(io, pending).await?;
            }
        }
        self.readiness()?;
        let mut report = tokio::time::interval(Duration::from_secs(5));
        let mut retry = tokio::time::interval(Duration::from_millis(100));
        let buffers = PacketPool::new(64, self.options.mtu, DEFAULT_HEADROOM, DEFAULT_TAILROOM)?;
        let mut batch = meshway_tun::PacketBatch::default();
        loop {
            tokio::select! {
                received = UDP_RECV.poll(io.recv()) => {
                    self.received(io, received?).await?;
                }
                read = async {
                    match &self.tun {
                        Some(tun) if self.ready => TUN_READ.poll(tun.recv_batch(&mut batch)).await,
                        _ => std::future::pending().await,
                    }
                } => {
                    read?;
                    let mut jobs = VecDeque::new();
                    for bytes in batch.take() {
                        let mut packet = buffers.acquire(bytes.len())?;
                        packet.copy_from_slice(&bytes);
                        if let Some(input) = self.outbound(packet)? { jobs.push_back(input); }
                    }
                    while !jobs.is_empty() {
                        let count = jobs.len().min(self.options.batch).min(32);
                        let mut group = jobs.drain(..count).collect();
                        let mut frames = Vec::new();
                        let outputs = self.pool.run_prepared(&mut group, |(circuit, packet)| {
                            let _packet = PACKET.enter();
                            self.node.prepare_send_packet(circuit, packet).map(Job::Send)
                        }).await;
                        for output in outputs {
                            match output {
                                Ok(Output::Send(frame)) => frames.push(frame),
                                Ok(Output::Receive(_)) => return Err("unexpected worker output".into()),
                                Err(error) if packet_rejection(&error) => self.rejected += 1,
                                Err(error) => return Err(error.into()),
                            }
                        }
                        if !frames.is_empty() { UDP_SEND.poll(io.send(frames, self.fixture)).await?; }
                    }
                }
                _ = retry.tick(), if !self.ready && matches!(self.options.role, NodeRole::A) => {
                    let pending_ids: Vec<_> = self.circuits.iter().filter(|(demand, _)| !self.established.contains_key(demand)).map(|(_, id)| *id).collect();
                    for id in pending_ids {
                        if let Some(frame) = self.node.retry_handshake(id)? { self.output(io, frame).await?; }
                    }
                }
                _ = report.tick() => profile::report(),
            }
            tokio::task::yield_now().await;
        }
    }
}
