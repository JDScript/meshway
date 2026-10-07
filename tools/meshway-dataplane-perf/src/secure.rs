use crate::{Error, NodeOptions, NodeRole, material};
use futures_util::FutureExt;
use meshway_dataplane::{
    DEFAULT_HEADROOM, DEFAULT_TAILROOM, Packet, PacketBuffer, PacketPool, carrier,
    secure::{CircuitAuthorization, Error as SecureError, PendingFrame, Received, SecureNode},
};
use meshway_logging::profile::{self, Metric};
use meshway_model::RequestId;
use meshway_static_authority::public_fixture::PublicFixture;
use meshway_transport::{Limits, Receiver, Sender};
use meshway_transport_spec::SocketOptions;
use meshway_transport_udp::UdpTransport;
use meshway_tun::{Options, Tun};
use std::{collections::BTreeMap, io::Write, path::Path, time::Duration};
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
    incoming: BTreeMap<std::net::SocketAddr, meshway_dataplane::secure::Carrier>,
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
    let incoming = fixture
        .carriers_for(local)?
        .into_iter()
        .map(|(id, carrier)| {
            Ok((
                *fixture.sockets.get(&id).ok_or("missing neighbour socket")?,
                carrier,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;
    let tun = if matches!(options.role, NodeRole::R) {
        None
    } else {
        Some(Tun::create(
            Options::new(options.mtu)?.with_ipv4(options.role.mesh_address(), 30)?,
        )?)
    };
    let socket = tokio::net::UdpSocket::bind(options.bind).await?;
    let endpoint = UdpTransport::adopt(socket, &SocketOptions::default())?;
    // Production Session budgets; additional capacity accounts for the real
    // secure envelope and handshake, never increases SecureNode's limits.
    let frame_limit = (options.mtu + 2048).min(65_507);
    let (mut sender, mut receiver) = endpoint
        .into_session(Limits::new(frame_limit, 1024, frame_limit * 1024, 1, 1024)?)
        .into_split();
    let mut driver = Driver {
        options,
        fixture: &fixture,
        node,
        incoming,
        tun,
        circuits: BTreeMap::new(),
        established: BTreeMap::new(),
        ready: false,
        unsupported: 0,
        rejected: 0,
    };
    let result = tokio::select! { biased;
        _ = stop.cancelled() => Ok(()),
        result = driver.run(&mut sender, &mut receiver) => result,
    };
    stop.cancel();
    sender.shutdown().await?;
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
    async fn output(&self, sender: &mut Sender, frame: PendingFrame) -> Result<(), Error> {
        let peer = self
            .fixture
            .sockets
            .get(&frame.next())
            .ok_or("unknown output next hop")?;
        // This official adapter releases at admission and retains the original
        // guard in SendContext through the actual UDP poll and completion.
        UDP_SEND
            .poll(carrier::send(sender, frame, Some(*peer)))
            .await?;
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
                "publicationDigest":self.fixture.publication_digest, "circuits":self.established.len()})
            );
            std::io::stdout().flush()?;
        }
        Ok(())
    }

    async fn received(
        &mut self,
        sender: &mut Sender,
        frame: meshway_transport::Frame,
    ) -> Result<(), Error> {
        let incoming = frame
            .peer
            .and_then(|peer| self.incoming.get(&peer))
            .ok_or("unexpected secure UDP source")?;
        let received = match self.node.receive(incoming, frame.payload) {
            Ok(received) => received,
            Err(error) if packet_rejection(&error) => {
                self.rejected += 1;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        match received {
            Received::Forward(frame) => self.output(sender, frame).await?,
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
                    self.output(sender, frame).await?;
                }
                self.readiness()?;
            }
            Received::Packet(action) => {
                let tun = self
                    .tun
                    .as_ref()
                    .ok_or("relay attempted plaintext delivery")?;
                TUN_WRITE.poll(carrier::deliver(tun, action)).await?;
                let _packet = DELIVER.enter();
            }
            _ => return Err("unexpected diagnostic message".into()),
        }
        Ok(())
    }

    async fn outbound(&mut self, sender: &mut Sender, bytes: PacketBuffer) -> Result<(), Error> {
        let Ok(packet) = Packet::parse(bytes) else {
            self.unsupported += 1;
            return Ok(());
        };
        let protocol = packet.facts().traffic.protocol;
        let Some(demand) = self
            .fixture
            .demands
            .values()
            .find(|demand| demand.protocol == protocol)
        else {
            self.unsupported += 1;
            return Ok(());
        };
        let circuit = *self
            .established
            .get(&demand.id)
            .ok_or("TUN read before circuit establishment")?;
        let frame = {
            let _packet = PACKET.enter();
            match self.node.send(circuit, packet) {
                Ok(frame) => frame,
                Err(error) if packet_rejection(&error) => {
                    self.rejected += 1;
                    return Ok(());
                }
                Err(error) => return Err(error.into()),
            }
        };
        self.output(sender, frame).await
    }

    async fn run(&mut self, sender: &mut Sender, receiver: &mut Receiver) -> Result<(), Error> {
        if matches!(self.options.role, NodeRole::A) {
            for demand in self.fixture.demands.keys() {
                let pending = self.node.start(*demand, self.fixture.path.clone())?;
                self.circuits.insert(*demand, pending.circuit());
                self.output(sender, pending).await?;
            }
        }
        self.readiness()?;
        let mut report = tokio::time::interval(Duration::from_secs(5));
        let mut retry = tokio::time::interval(Duration::from_millis(100));
        let buffers = PacketPool::new(64, self.options.mtu, DEFAULT_HEADROOM, DEFAULT_TAILROOM)?;
        let mut buffer = buffers.acquire(self.options.mtu)?;
        loop {
            tokio::select! {
                received = UDP_RECV.poll(receiver.recv()) => {
                    self.received(sender, received?).await?;
                    for _ in 1..self.options.batch {
                        let Some(frame) = UDP_RECV.poll(receiver.recv()).now_or_never() else { break; };
                        self.received(sender, frame?).await?;
                    }
                }
                read = async {
                    match &self.tun {
                        Some(tun) if self.ready => TUN_READ.poll(tun.recv(&mut buffer)).await,
                        _ => std::future::pending().await,
                    }
                } => {
                    buffer.truncate(read?);
                    let packet = std::mem::replace(&mut buffer, buffers.acquire(self.options.mtu)?);
                    self.outbound(sender, packet).await?;
                    for _ in 1..self.options.batch {
                        let tun = self.tun.as_ref().ok_or("missing endpoint TUN")?;
                        let Some(read) = TUN_READ.poll(tun.recv(&mut buffer)).now_or_never() else { break; };
                        buffer.truncate(read?);
                        let packet = std::mem::replace(&mut buffer, buffers.acquire(self.options.mtu)?);
                        self.outbound(sender, packet).await?;
                    }
                }
                _ = retry.tick(), if !self.ready && matches!(self.options.role, NodeRole::A) => {
                    let pending_ids: Vec<_> = self.circuits.iter().filter(|(demand, _)| !self.established.contains_key(demand)).map(|(_, id)| *id).collect();
                    for id in pending_ids {
                        if let Some(frame) = self.node.retry_handshake(id)? { self.output(sender, frame).await?; }
                    }
                }
                _ = report.tick() => profile::report(),
            }
            tokio::task::yield_now().await;
        }
    }
}
