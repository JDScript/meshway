//! Platform-neutral, IPv4-only endpoint packet I/O. Platform route application and
//! coherent authorization compilation remain the coordinator's responsibility.

use std::{collections::BTreeMap, future::Future, io, net::Ipv4Addr, sync::Arc};

use tokio::sync::{Mutex, mpsc, watch};
use tokio::time::Instant;

use crate::{MAX_BATCH_BYTES, MAX_BATCH_FRAMES, MAX_PACKET_PAYLOAD, ReceivedPacket, Runtime};

/// Owned plaintext packets, in callback order. Debug deliberately omits bytes.
pub struct PacketBatch(Vec<Vec<u8>>);

impl std::fmt::Debug for PacketBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacketBatch")
            .field("packets", &self.0.len())
            .finish()
    }
}

impl PacketBatch {
    pub fn new(packets: Vec<Vec<u8>>) -> io::Result<Self> {
        if packets.is_empty()
            || packets.len() > MAX_BATCH_FRAMES
            || packets
                .iter()
                .any(|p| p.is_empty() || p.len() > MAX_PACKET_PAYLOAD)
            || packets.iter().map(Vec::len).sum::<usize>() > MAX_BATCH_BYTES
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "packet batch exceeds bounds",
            ));
        }
        Ok(Self(packets))
    }

    pub fn into_packets(self) -> Vec<Vec<u8>> {
        self.0
    }
}

/// One duplex device, shared by exactly one pump. Futures must be cancellation
/// safe: dropping a receive/write must not leave a background write or task.
/// A successful write means local acceptance only. Implementations may receive
/// single packets; batches are bounded by PacketBatch, with no boxed futures.
pub trait Device: Send + Sync {
    fn recv_batch(&self) -> impl Future<Output = io::Result<PacketBatch>> + Send;
    fn send_batch(&self, packets: PacketBatch) -> impl Future<Output = io::Result<()>> + Send;
    /// Nonblocking bounded discard before activating a replacement generation.
    fn drain(&self) -> io::Result<()>;
}

/// A platform MUST capture this prefix and keep it fail-closed when packet I/O
/// stops (including process/extension termination). It must never fall through
/// to the underlay. No IPv6 routes, DNS, forwarding or default route are implied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutePolicy {
    pub local: Ipv4Addr,
    pub network: Ipv4Addr,
    pub prefix_len: u8,
    pub mtu: u16,
    pub retain_fail_closed_sink: bool,
}

/// Immutable admission for one already-validated coherent generation. Paths
/// are exact directional grants, never inferred from adjacency or reversed.
#[derive(Clone)]
pub struct Ipv4PacketPlan {
    local: Ipv4Addr,
    mtu: u16,
    outgoing: BTreeMap<Ipv4Addr, Vec<String>>,
    incoming: BTreeMap<Vec<String>, Ipv4Addr>,
}

impl Ipv4PacketPlan {
    pub fn new(
        local: Ipv4Addr,
        mtu: u16,
        outgoing: BTreeMap<Ipv4Addr, Vec<String>>,
        incoming: BTreeMap<Vec<String>, Ipv4Addr>,
    ) -> Self {
        Self {
            local,
            mtu,
            outgoing,
            incoming,
        }
    }

    pub fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        let (source, destination) = ipv4(bytes, self.mtu)?;
        if source != self.local {
            return None;
        }
        self.outgoing.get(&destination)
    }

    pub fn incoming(&self, packet: &ReceivedPacket) -> bool {
        packet.path.first() == Some(&packet.source)
            && self.incoming.get(&packet.path).is_some_and(|source| {
                ipv4(&packet.payload, self.mtu) == Some((*source, self.local))
            })
    }
}

/// Generation-scoped packet admission. Stateful implementations must keep
/// bounded state and may not infer authentication from inner packet addresses.
pub trait Admission: Send + Sync {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>>;
    fn incoming(&self, packet: &ReceivedPacket) -> bool;
    /// Endpoint-only envelope; Runtime authenticates and encrypts these bytes.
    fn encapsulate(&self, _path: &[String], bytes: &[u8]) -> io::Result<Vec<u8>> {
        Ok(bytes.to_vec())
    }
    /// Called only after Runtime accepted the send, never on failure/cancellation.
    fn sent(&self, _path: &[String], _payload: &[u8]) -> io::Result<()> {
        Ok(())
    }
    fn plaintext(&self, packet: &ReceivedPacket) -> io::Result<Vec<u8>> {
        Ok(packet.payload.clone())
    }
    /// Called only after the device accepted the complete authenticated packet.
    fn delivered(&self, _packet: &ReceivedPacket) -> io::Result<()> {
        Ok(())
    }
    /// Authenticated endpoint control, consumed without injecting into the device.
    fn control(&self, _packet: &ReceivedPacket) -> io::Result<bool> {
        Ok(false)
    }
    /// Receipt generated only after complete device acceptance. Carries no IP data.
    fn receipt(&self, _packet: &ReceivedPacket) -> Option<Vec<u8>> {
        None
    }
}

impl Admission for Ipv4PacketPlan {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        self.outgoing(bytes)
    }
    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        self.incoming(packet)
    }
}

/// Strict complete IPv4 packets, without options or fragments.
pub fn ipv4(bytes: &[u8], mtu: u16) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if bytes.len() < 20
        || bytes.len() > usize::from(mtu)
        || bytes[0] != 0x45
        || usize::from(u16::from_be_bytes([bytes[2], bytes[3]])) != bytes.len()
        || u16::from_be_bytes([bytes[6], bytes[7]]) & 0xbfff != 0
        || bytes[8] == 0
    {
        return None;
    }
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    if sum != 0xffff {
        return None;
    }
    Some((
        Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]),
        Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]),
    ))
}

fn current(deadline: &watch::Receiver<Option<Instant>>) -> bool {
    deadline
        .borrow()
        .is_some_and(|until| until > Instant::now())
}

/// RAII preserves the unique delivery channel across admission-only selection.
/// Runtime shutdown still owns cryptographic/session retirement. Bounded discard
/// occurs before activation so buffered old plaintext cannot cross a generation.
struct ReceiverLease {
    runtime: Arc<Runtime>,
    receiver: Option<mpsc::Receiver<ReceivedPacket>>,
    generation: Option<crate::runtime::generation::Generation>,
}

impl ReceiverLease {
    fn take(runtime: Arc<Runtime>) -> io::Result<Self> {
        let mut receiver = runtime
            .take_packet_receiver()
            .map_err(|_| io::Error::other("runtime packet receiver unavailable"))?;
        runtime.retire_packet_generation();
        for _ in 0..256 {
            match receiver.try_recv() {
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => {
                    return Ok(Self {
                        generation: Some(runtime.begin_packet_generation()),
                        runtime,
                        receiver: Some(receiver),
                    });
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    return Err(io::Error::other("packet runtime closed"));
                }
            }
        }
        // A permanently busy old queue is a failed prepare, never activated.
        runtime.return_packet_receiver(receiver);
        Err(io::Error::other("old runtime packet queue did not quiesce"))
    }

    async fn recv(&mut self) -> Option<ReceivedPacket> {
        self.receiver.as_mut().expect("owned receiver").recv().await
    }
}

impl Drop for ReceiverLease {
    fn drop(&mut self) {
        // Fence queued ingress/outbound and blocked carrier polls before making
        // the unique receiver available to a replacement admission generation.
        self.generation.take();
        if let Some(receiver) = self.receiver.take() {
            self.runtime.return_packet_receiver(receiver);
        }
    }
}

/// Single bounded duplex pump. Drop this future to cancel both directions;
/// lease expiry, withdrawal and a closed lease sender also cancel blocked I/O.
/// The coordinator must terminate the old pump before draining/reusing a device.
/// Joining or dropping the pump retires its local Runtime packet queues and
/// synchronizes with carrier I/O polls. Partially written streams are closed;
/// E2E caches survive. Already emitted network bytes cannot be recalled. Ingress
/// is attributed at authenticated carrier delivery, then fenced through dispatch;
/// queued old delivery is discarded before the next activation.
pub async fn run<D: Device, A: Admission>(
    device: Arc<D>,
    runtime: Arc<Runtime>,
    plan: A,
    deadline: watch::Receiver<Option<Instant>>,
) -> io::Result<()> {
    run_with_activation(device, runtime, plan, deadline, || {}).await
}

/// As `run`, with a bounded synchronous observation after acquiring the unique
/// runtime receiver and checking the live lease, before either I/O direction.
/// The callback is a local activation observation, never a delivery proof.
pub async fn run_with_activation<D: Device, A: Admission>(
    device: Arc<D>,
    runtime: Arc<Runtime>,
    plan: A,
    mut deadline: watch::Receiver<Option<Instant>>,
    activated: impl FnOnce() + Send,
) -> io::Result<()> {
    let mut received = ReceiverLease::take(runtime.clone())?;
    if !current(&deadline) {
        return Ok(());
    }
    activated();
    let io_deadline = deadline.clone();
    let outbound = async {
        loop {
            let batch = device.recv_batch().await?;
            for bytes in batch.into_packets() {
                if !current(&io_deadline) {
                    return Ok(());
                }
                if let Some(path) = plan.outgoing(&bytes) {
                    let payload = plan.encapsulate(path, &bytes)?;
                    match runtime.send_packet(path.clone(), payload.clone()).await {
                        Ok(()) => plan.sent(path, &payload)?,
                        Err(
                            crate::Error::Unavailable
                            | crate::Error::Capacity
                            | crate::Error::Timeout,
                        ) => {}
                        Err(_) => return Err(io::Error::other("packet runtime failure")),
                    }
                }
                // Preserve the managed pump's per-packet withdrawal fairness.
                tokio::task::yield_now().await;
            }
        }
    };
    let inbound = async {
        loop {
            let packet = received
                .recv()
                .await
                .ok_or_else(|| io::Error::other("packet runtime closed"))?;
            if !current(&io_deadline) {
                return Ok(());
            }
            if plan.control(&packet)? {
                tokio::task::yield_now().await;
                continue;
            }
            if plan.incoming(&packet) {
                device
                    .send_batch(PacketBatch::new(vec![plan.plaintext(&packet)?])?)
                    .await?;
                if let Some(receipt) = plan.receipt(&packet) {
                    let reverse = packet.path.iter().rev().cloned().collect();
                    match runtime.send_packet(reverse, receipt).await {
                        Ok(()) => {}
                        Err(
                            crate::Error::Unavailable
                            | crate::Error::Capacity
                            | crate::Error::Timeout,
                        ) => {
                            tokio::task::yield_now().await;
                            continue;
                        }
                        Err(_) => return Err(io::Error::other("packet receipt runtime failure")),
                    }
                }
                plan.delivered(&packet)?;
            }
            tokio::task::yield_now().await;
        }
    };
    let withdrawn = async {
        loop {
            let Some(until) = *deadline.borrow_and_update() else {
                return;
            };
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(until) => return,
                result = deadline.changed() => if result.is_err() { return; },
            }
        }
    };
    tokio::select! {
        biased;
        _ = withdrawn => Ok(()),
        result = async { tokio::try_join!(outbound, inbound).map(|_| ()) } => result,
    }
}

/// Callback ingress and async callback egress. Queues hold bounded *batches*,
/// not packets; at most capacity * 65,536 bytes per direction plus in-flight I/O.
pub struct ChannelDevice {
    input: Mutex<mpsc::Receiver<PacketBatch>>,
    output: mpsc::Sender<PacketBatch>,
}

pub struct ChannelEndpoint {
    input: mpsc::Sender<PacketBatch>,
    output: Mutex<mpsc::Receiver<PacketBatch>>,
}

pub fn channel(capacity: usize) -> io::Result<(Arc<ChannelDevice>, ChannelEndpoint)> {
    if !(1..=256).contains(&capacity) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "packet queue capacity must be 1–256",
        ));
    }
    let (input, receive) = mpsc::channel(capacity);
    let (send, output) = mpsc::channel(capacity);
    Ok((
        Arc::new(ChannelDevice {
            input: Mutex::new(receive),
            output: send,
        }),
        ChannelEndpoint {
            input,
            output: Mutex::new(output),
        },
    ))
}

impl ChannelEndpoint {
    /// Nonblocking atomic admission. Full/closed returns the owned batch.
    pub fn try_feed(
        &self,
        packets: PacketBatch,
    ) -> Result<(), mpsc::error::TrySendError<PacketBatch>> {
        self.input.try_send(packets)
    }

    pub async fn recv_batch(&self) -> Option<PacketBatch> {
        self.output.lock().await.recv().await
    }

    /// Call only after stopping the pump and callback writer. Discard both queues
    /// before reusing this endpoint; otherwise prefer a fresh channel per generation.
    pub fn drain(&self) -> io::Result<()> {
        drain(&self.output)
    }
}

fn drain(queue: &Mutex<mpsc::Receiver<PacketBatch>>) -> io::Result<()> {
    let mut receiver = queue
        .try_lock()
        .map_err(|_| io::Error::other("concurrent packet I/O during drain"))?;
    for _ in 0..256 {
        match receiver.try_recv() {
            Ok(_) => {}
            Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
            Err(_) => return Err(io::Error::other("packet device closed")),
        }
    }
    if receiver.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other("packet queue did not quiesce"))
    }
}

impl Device for ChannelDevice {
    async fn recv_batch(&self) -> io::Result<PacketBatch> {
        self.input
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| io::Error::other("packet device closed"))
    }
    async fn send_batch(&self, packets: PacketBatch) -> io::Result<()> {
        self.output
            .send(packets)
            .await
            .map_err(|_| io::Error::other("packet device closed"))
    }
    fn drain(&self) -> io::Result<()> {
        drain(&self.input)
    }
}
