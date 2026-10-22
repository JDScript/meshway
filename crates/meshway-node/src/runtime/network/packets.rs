//! Bounded scheduling of original packets. There is no second flow table:
//! every local packet is decided by SecureNode's unique owner on the
//! installed netmap, with no Server round trip.
use meshway_connectivity::{Carrier, Manager};
use meshway_dataplane::{
    Packet, Purpose,
    secure::{self, CircuitState, Flushed, Outbound, Received, SecureNode},
};
use meshway_engine::Desired;
use meshway_model::NodeId;
use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};
use tokio::time::Instant;

const MAX_PACKETS: usize = 128;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ATTEMPTS: usize = 64;
/// How long queued first packets are polled for a ready adjacency or circuit.
const FIRST_PACKET_POLL: Duration = Duration::from_secs(10);
const RETENTION: Duration = Duration::from_secs(20);
const MAX_SENDS: usize = 32;

struct Queued {
    packet: Packet,
    deadline: Instant,
}
struct Incoming {
    carrier: Carrier,
    link: meshway_connectivity::LinkFact,
    received_at: std::time::Instant,
    bytes: meshway_dataplane::PacketBuffer,
    deadline: Instant,
}
struct Attempt {
    circuit: [u8; 32],
    wake_at: Instant,
}

struct Delivered {
    count: usize,
}

#[derive(Default)]
pub(super) struct Packets {
    local: VecDeque<Queued>,
    incoming: VecDeque<Incoming>,
    control: VecDeque<Incoming>,
    attempts: Vec<Attempt>,
    /// First packets wait inside SecureNode; poll its flush until then.
    first_until: Option<Instant>,
    traced_circuits: BTreeMap<[u8; 32], Instant>,
    pub dropped: u64,
    carrier_wait_logged: Option<Instant>,
    held_logged: Option<Instant>,
    scan_remaining: usize,
    sends:
        meshway_dataplane::carrier::SendPending<(Result<(), meshway_connectivity::Error>, usize)>,
    deliveries: meshway_dataplane::carrier::TunPending<Delivered>,
    exit_replies: VecDeque<secure::PendingFrame>,
    /// Closes of circuits a restarted adjacent peer lost, bounded.
    closes: Vec<secure::PendingFrame>,
}

impl Packets {
    pub fn exit_reply(&mut self, frame: secure::PendingFrame) {
        if self.exit_replies.len() < MAX_PACKETS {
            self.exit_replies.push_back(frame);
        } else {
            self.drop_packet("exit_reply_capacity");
        }
    }
    /// Room for one more batch of exit replies; otherwise they wait in the
    /// exit TUN's queue instead of being dropped here.
    pub fn can_take_exit_replies(&self, batch: usize) -> bool {
        self.exit_replies.len() + batch <= MAX_PACKETS
    }
    pub fn closes(&mut self, frames: Vec<secure::PendingFrame>) {
        for frame in frames {
            if self.closes.len() < MAX_SENDS {
                self.closes.push(frame);
            } else {
                self.drop_packet("close_capacity");
            }
        }
    }

    pub fn can_read(&self, pool: &secure::pipeline::Pool) -> bool {
        pool.can_submit()
            && self.local.len() + self.incoming.len() <= MAX_PACKETS - 32
            && self.bytes() <= MAX_BYTES - 32 * 4096
    }

    pub fn has_outputs(&self) -> bool {
        !self.sends.is_empty() || !self.deliveries.is_empty()
    }

    /// Cancellation-safe: the queue retains its future / unwritten TUN suffix.
    /// Both output directions advance independently of crypto and input readiness.
    pub async fn sent(&mut self, host: &dyn crate::host::Host) {
        tokio::select! {
            (result, count) = self.sends.ready(), if !self.sends.is_empty() => {
                if result.is_err() {
                    for _ in 0..count { self.drop_packet("batch_send"); }
                }
            }
            (result, delivered) = async {
                match host.tun() {
                    Some(tun) => self.deliveries.ready(tun).await,
                    None => std::future::pending().await,
                }
            }, if !self.deliveries.is_empty() => {
                if result.is_err() {
                    for _ in 0..delivered.count { self.drop_packet("tun_delivery"); }
                }
            }
        }
    }

    fn trace_packet(&mut self, frame: &secure::PendingFrame, circuit: &secure::CircuitInfo) {
        const TARGET: &str = "meshway_node::runtime::network::circuit_packet";
        if !tracing::enabled!(target: TARGET, tracing::Level::DEBUG) {
            return;
        }
        // This observation is emitted only for admitted OS business packets.
        // Diagnostic observations have their own typed measurement events.
        if circuit.grant.purpose != Purpose::Business {
            return;
        }
        let target = &circuit.grant.target;
        let now = Instant::now();
        let id = frame.circuit();
        if self
            .traced_circuits
            .get(&id)
            .is_some_and(|previous| now.duration_since(*previous) < Duration::from_secs(1))
        {
            return;
        }
        if self.traced_circuits.len() >= MAX_ATTEMPTS {
            self.traced_circuits
                .retain(|_, previous| now.duration_since(*previous) < Duration::from_secs(5));
            if !self.traced_circuits.contains_key(&id) && self.traced_circuits.len() >= MAX_ATTEMPTS
            {
                return;
            }
        }
        let Ok(path_json) = serde_json::to_string(&circuit.path) else {
            return;
        };
        let Ok(observed_at_ms) = u64::try_from(chrono::Utc::now().timestamp_millis()) else {
            return;
        };
        self.traced_circuits.insert(id, now);
        let cid = id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        tracing::debug!(target: TARGET, %cid, ?target, %path_json, observed_at_ms,
            "Mesh circuit packet ready");
    }
    /// At most once a second while local packets wait after a turn (in this
    /// queue, as first packets inside SecureNode, or unread in the TUN):
    /// where they wait.
    fn trace_held(
        &mut self,
        secure: &SecureNode,
        pool: &secure::pipeline::Pool,
        incoming_done: usize,
        local_done: usize,
    ) {
        let first = secure.first_packets_waiting();
        let can_read = self.can_read(pool);
        if (self.local.is_empty() && first == 0 && can_read)
            || self
                .held_logged
                .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.held_logged = Some(Instant::now());
        let oldest_local_ms = self.local.front().map(|item| {
            u64::try_from((item.deadline - RETENTION).elapsed().as_millis()).unwrap_or(u64::MAX)
        });
        tracing::debug!(
            local = self.local.len(),
            oldest_local_ms,
            first_waiting = first,
            can_read,
            incoming = self.incoming.len(),
            control = self.control.len(),
            local_done,
            incoming_done,
            pool_available = pool.available(),
            sends = self.sends.occupancy().0,
            deliveries = self.deliveries.occupancy().0,
            "Local packets held after a turn"
        );
    }
    pub fn wake_at(&self) -> Option<Instant> {
        let now = Instant::now();
        if self.scan_remaining != 0 || !self.closes.is_empty() {
            return Some(now);
        }
        let first = self
            .first_until
            .filter(|until| *until > now)
            .map(|_| now + Duration::from_millis(100));
        self.local
            .iter()
            .map(|item| item.deadline)
            .chain(self.incoming.iter().map(|item| item.deadline))
            .chain(self.control.iter().map(|item| item.deadline))
            .chain(self.attempts.iter().map(|attempt| attempt.wake_at))
            .chain(first)
            .min()
    }
    /// New content: outputs admitted on earlier content end. Frames not yet
    /// admitted stay queued; SecureNode admits each on the content current
    /// when it is processed.
    pub fn clear_admitted(&mut self) {
        self.dropped = self
            .dropped
            .saturating_add((self.sends.occupancy().0 + self.deliveries.occupancy().0) as u64);
        self.sends = Default::default();
        self.deliveries = Default::default();
        self.exit_replies.clear();
    }
    fn bytes(&self) -> usize {
        self.local
            .iter()
            .map(|item| item.packet.as_bytes().len())
            .sum::<usize>()
            + self
                .incoming
                .iter()
                .map(|item| item.bytes.len())
                .sum::<usize>()
    }
    pub fn drop_packet(&mut self, reason: &'static str) {
        meshway_logging::profile::event("host.drop", reason);
        self.dropped = self.dropped.saturating_add(1);
        // Logarithmic sampling remains bounded under malformed/unsupported input.
        if self.dropped.is_power_of_two() {
            tracing::debug!(reason, dropped = self.dropped, "Packet discarded");
        }
    }

    fn drop_admission(&mut self, reason: &'static str, error: secure::Error) {
        self.dropped = self.dropped.saturating_add(1);
        meshway_logging::profile::event("host.reject", reason);
        meshway_logging::profile::event(
            "host.reject.kind",
            match error {
                secure::Error::Flow => "flow",
                secure::Error::Authority => "authority",
                secure::Error::Expired => "expired",
                secure::Error::Policy => "policy",
                secure::Error::Replay => "replay",
                secure::Error::Proof => "proof",
                secure::Error::TooBig => "too_big",
                secure::Error::Ipv6Unavailable => "ipv6_unavailable",
                _ => "other",
            },
        );
        if self.dropped.is_power_of_two() {
            tracing::debug!(reason, ?error, dropped = self.dropped, "Packet discarded");
        }
    }
    pub fn enqueue(&mut self, packet: Packet) {
        if self.local.len() + self.incoming.len() >= MAX_PACKETS
            || self.bytes() + packet.as_bytes().len() > MAX_BYTES
        {
            self.drop_packet("queue_capacity");
            return;
        }
        self.local.push_back(Queued {
            packet,
            deadline: Instant::now() + RETENTION,
        });
    }

    pub fn incoming(
        &mut self,
        link: meshway_connectivity::LinkFact,
        bytes: meshway_dataplane::PacketBuffer,
        received_at: std::time::Instant,
        desired: &Desired,
        secure: &SecureNode,
    ) {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("host.incoming");
        let _profile = PROFILE.enter();
        let local = desired.netmap().node_id;
        // Hints only decide bounded scheduling. receive() authenticates packet
        // content, this verifier's full path prefix and the actual carrier;
        // downstream MAC columns are checked by their respective verifiers.
        let Ok(hint) = secure.incoming_hint(&bytes) else {
            self.drop_packet("cipher_header");
            return;
        };
        // Scheduling only: receive() decides the path on the netmap.
        let known = hint.path.contains(&local);
        if hint.grant.purpose == Purpose::Diagnostic {
            tracing::debug!(grant_untrusted = ?hint.grant, %local,
                netmap_serial = desired.reference().serial.get(),
                known, "Diagnostic carrier frame received for scheduling");
        }
        let control = !secure::is_data_frame(&bytes);
        let full = if control {
            self.control.len() >= 32
        } else {
            self.local.len() + self.incoming.len() >= MAX_PACKETS
                || self.bytes() + bytes.len() > MAX_BYTES
        };
        if !known || bytes.len() > secure::MAX_FRAME || full {
            meshway_logging::profile::event(
                "host.cipher.reject",
                if !known {
                    "unknown_scope"
                } else {
                    "capacity_or_size"
                },
            );
            self.drop_packet("cipher_context_or_capacity");
            return;
        }
        let queue = if control {
            &mut self.control
        } else {
            &mut self.incoming
        };
        queue.push_back(Incoming {
            carrier: link.carrier.clone(),
            link,
            received_at,
            bytes,
            deadline: Instant::now() + Duration::from_secs(5),
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn drive(
        &mut self,
        secure: &mut SecureNode,
        manager: &Manager,
        carriers: Option<&BTreeMap<NodeId, Carrier>>,
        host: &dyn crate::host::Host,
        desired: &Desired,
        diagnostics: &mut super::diagnostics::Diagnostics,
        dns_queries: &mut super::dns::mesh::Queries,
        dns: Option<std::sync::Arc<meshway_dns::OwnerDispatcher>>,
        pool: &mut secure::pipeline::Pool,
        mut exit: Option<&mut crate::host::ExitAttachment>,
    ) {
        let local = desired.netmap().node_id;
        let now = Instant::now();
        let Some(carriers) = carriers else {
            return;
        };
        // `meshway ping` echoes outside the Mesh enter as local packets.
        for packet in diagnostics
            .externals
            .packets(desired.local().addresses.ipv4)
        {
            self.enqueue(packet);
        }
        if let Some(exit) = exit.as_deref_mut() {
            exit.drive(secure);
        }
        let mut replies = Vec::new();
        for _ in 0..MAX_SENDS {
            let Some(frame) = self.exit_replies.pop_front() else {
                break;
            };
            // The same worker must keep draining projection intents when the
            // driver FIFO was full. Never await its ACK while monopolizing that
            // worker; keep the original frame and poll it on the next turn.
            let ready = {
                use std::future::Future;
                let mut ready = std::pin::pin!(frame.projection_ready());
                std::future::poll_fn(|cx| std::task::Poll::Ready(ready.as_mut().poll(cx))).await
            };
            match ready {
                std::task::Poll::Pending => {
                    self.exit_replies.push_front(frame);
                    break;
                }
                std::task::Poll::Ready(Ok(())) => replies.push(frame),
                std::task::Poll::Ready(Err(_)) => self.drop_packet("exit_reply_expired"),
            }
        }
        // One batched send per turn, as for Mesh frames.
        self.flush_sends(manager, &mut replies).await;
        let ended = secure.take_exit_closes();
        self.closes(ended);
        let mut closes = std::mem::take(&mut self.closes);
        self.flush_sends(manager, &mut closes).await;
        for keepalive in secure.keepalives(pool.available().min(MAX_SENDS)) {
            match keepalive {
                Ok(job) => {
                    assert!(pool.submit(secure::pipeline::Job::Send(job)).is_none());
                }
                Err(error) => self.drop_admission("keepalive", error),
            }
        }
        self.complete_crypto(
            secure,
            manager,
            pool,
            exit.as_deref_mut(),
            &mut diagnostics.externals,
        );
        // Continue a bounded scan on the next turn, including ready suffixes
        // left after a full batch. Waiting packets are scanned only once per
        // external event, rather than creating an unbounded busy loop.
        if self.scan_remaining == 0 {
            self.scan_remaining = self.local.len() + self.incoming.len() + self.control.len();
        }
        let per_direction = if self.local.is_empty() || self.incoming.is_empty() {
            32
        } else {
            16
        };
        let incoming_count = (self.incoming.len() + self.control.len()).min(per_direction);
        let local_count = self.local.len().min(per_direction);
        let mut incoming_done = 0;
        let mut local_done = 0;
        let quantum = std::time::Instant::now();
        let mut outgoing = Vec::new();
        let mut ready = Vec::new();
        for _ in 0..incoming_count {
            let Some(item) = self
                .control
                .pop_front()
                .or_else(|| self.incoming.pop_front())
            else {
                break;
            };
            incoming_done += 1;
            if secure::is_data_frame(&item.bytes)
                && (!pool.can_submit_capacity(item.bytes.capacity())
                    || quantum.elapsed() >= Duration::from_micros(200))
            {
                self.incoming.push_front(item);
                break;
            }
            // Queues are cleared by every apply; only their own bound remains.
            if item.deadline <= Instant::now() {
                self.drop_packet("cipher_deadline");
                continue;
            }
            let hint = secure.incoming_hint(&item.bytes).ok();
            let waiting = hint.as_ref().and_then(|hint| {
                let previous = if item.carrier.dialer == local {
                    item.carrier.listener
                } else {
                    item.carrier.dialer
                };
                hint.next_hop(local, previous)
                    .filter(|node| !carriers.contains_key(node))
            });
            if let Some(peer) = waiting {
                // The authenticated incoming channel may need another allowed
                // adjacency. This is only an acquisition hint: Manager checks
                // that edge, and SecureNode still authenticates the full frame
                // and original path before forwarding any payload.
                use meshway_connectivity::AdjacencyInterest;
                manager.interest().interest(peer);
                if self
                    .carrier_wait_logged
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
                {
                    self.carrier_wait_logged = Some(Instant::now());
                    tracing::debug!(grant_untrusted = ?hint.as_ref().map(|hint| &hint.grant), path_untrusted = ?hint.as_ref().map(|hint| &hint.path), %local,
                        netmap_serial = desired.reference().serial.get(),
                        available = ?carriers.keys().collect::<Vec<_>>(), "Waiting for authenticated next-hop carrier before secure receive");
                }
                if secure::is_data_frame(&item.bytes) {
                    self.incoming.push_back(item);
                } else {
                    self.control.push_back(item);
                }
                continue;
            }
            if meshway_logging::profile::enabled() {
                static WAIT: meshway_logging::profile::Metric =
                    meshway_logging::profile::Metric::new("queue.manager_to_secure");
                WAIT.elapsed(item.received_at.elapsed());
            }
            let result = if secure::is_data_frame(&item.bytes) {
                match secure.prepare_receive(&item.carrier, item.bytes) {
                    Ok(job) => {
                        assert!(pool.submit(secure::pipeline::Job::Receive(job)).is_none());
                        if pool.available().is_multiple_of(8) {
                            pool.flush();
                        }
                        continue;
                    }
                    Err(error) => Some(Err(error)),
                }
            } else {
                Some(secure.receive(&item.carrier, item.bytes))
            };
            ready.push((item.link, item.received_at, hint, result));
        }
        pool.flush();
        let mut deliveries = Vec::new();
        for (link, received_at, hint, result) in ready {
            let result = result.expect("synchronous control result");
            match result {
                Ok(Received::DnsRequest(request)) => dns_queries.receive(request, dns.clone()),
                Ok(Received::DnsReply(reply)) => dns_queries.reply(reply),
                Ok(Received::Forward(frame))
                | Ok(Received::Established {
                    response: Some(frame),
                    ..
                }) => {
                    let sent = if hint
                        .as_ref()
                        .is_some_and(|hint| hint.grant.purpose == Purpose::Diagnostic)
                    {
                        diagnostics.forward(manager, frame)
                    } else {
                        outgoing.push(frame);
                        Ok(())
                    };
                    if sent.is_err() {
                        self.drop_packet("cipher_send");
                    }
                    if outgoing.len() == MAX_SENDS {
                        self.flush_sends(manager, &mut outgoing).await;
                    }
                }
                Ok(
                    Received::Established { response: None, .. }
                    | Received::Keepalive { .. }
                    | Received::Closed { .. },
                ) => {}
                Ok(Received::ProbeRequest {
                    circuit,
                    measurement_id,
                    response,
                }) => {
                    tracing::debug!(?circuit, %measurement_id, netmap_serial = desired.reference().serial.get(), "Authenticated diagnostic request received");
                    if diagnostics.forward(manager, response).is_err() {
                        self.drop_packet("probe_reply_send");
                    }
                }
                Ok(Received::ProbeReply {
                    circuit,
                    measurement_id,
                    challenge,
                }) => {
                    tracing::debug!(?circuit, %measurement_id, netmap_serial = desired.reference().serial.get(), "Authenticated diagnostic reply received");
                    if let Ok(channel) = super::diagnostics::channel(&link) {
                        diagnostics.owner.reply(
                            circuit,
                            measurement_id,
                            challenge,
                            channel,
                            received_at,
                        );
                    }
                }
                Ok(Received::Packet(packet)) => {
                    if packet.hop() == meshway_dataplane::Hop::Exit {
                        if let Some(exit) = exit.as_deref_mut() {
                            exit.packet(secure, packet);
                        } else if let Some(flow) = packet.exit_flow() {
                            secure.fail_exit_projection(&flow);
                        }
                        continue;
                    }
                    if packet.hop() != meshway_dataplane::Hop::Deliver {
                        self.drop_packet("unsupported_exit");
                        continue;
                    }
                    if diagnostics.externals.answer(&packet) {
                        continue;
                    }
                    deliveries.push(packet);
                }
                Err(error) => self.drop_admission("cipher_admission", error),
            }
        }
        self.flush_sends(manager, &mut outgoing).await;
        if !deliveries.is_empty()
            && let Some(tun) = host.tun()
        {
            let count = deliveries.len();
            if meshway_dataplane::carrier::deliver_batch(tun, deliveries)
                .await
                .is_err()
            {
                for _ in 0..count {
                    self.drop_packet("tun_delivery");
                }
            }
        }
        self.attempts.retain_mut(|attempt| {
            secure
                .circuit_info(attempt.circuit)
                .ok()
                .flatten()
                .is_some_and(|info| {
                    attempt.wake_at = secure
                        .handshake_retry_at(attempt.circuit)
                        .ok()
                        .flatten()
                        .map_or(info.expires_at.into(), |at| at.min(info.expires_at).into());
                    info.state != CircuitState::Closed
                })
        });
        for attempt in &mut self.attempts {
            if attempt.wake_at > Instant::now() {
                continue;
            }
            // The unique SecureNode owns cached INIT bytes, original guards,
            // total deadline and retry budget. A local send failure does not
            // rebuild Noise or close that still-live handshake.
            if let Ok(Some(frame)) = secure.retry_handshake(attempt.circuit)
                && manager.send(frame).await.is_err()
            {
                self.dropped = self.dropped.saturating_add(1);
            }
            if let Ok(Some(info)) = secure.circuit_info(attempt.circuit) {
                attempt.wake_at = secure
                    .handshake_retry_at(attempt.circuit)
                    .ok()
                    .flatten()
                    .map_or(info.expires_at.into(), |at| at.min(info.expires_at).into());
            }
        }
        for _ in 0..local_count {
            if self.local.front().is_some_and(|item| {
                !pool.can_submit_capacity(
                    item.packet.as_bytes().len()
                        + meshway_dataplane::DEFAULT_HEADROOM
                        + meshway_dataplane::DEFAULT_TAILROOM,
                )
            }) || quantum.elapsed() >= Duration::from_micros(200)
            {
                break;
            }
            let Some(item) = self.local.pop_front() else {
                break;
            };
            local_done += 1;
            if item.deadline <= Instant::now() {
                self.drop_packet("packet_deadline");
                continue;
            }
            if meshway_logging::profile::enabled() {
                static WAIT: meshway_logging::profile::Metric =
                    meshway_logging::profile::Metric::new("queue.tun_attempt_age");
                WAIT.elapsed((item.deadline - RETENTION).elapsed());
            }
            let echo = diagnostics.externals.request(&item.packet.facts());
            let result = secure.outbound(item.packet);
            if let Some(echo) = echo {
                let admitted = match &result {
                    Ok(Outbound::Send(_)) => Some(false),
                    Ok(Outbound::Queued { .. } | Outbound::AwaitingAdjacency) => Some(true),
                    _ => None,
                };
                diagnostics.externals.admitted(echo, admitted);
            }
            match result {
                Ok(Outbound::Send(job)) => {
                    assert!(pool.submit(secure::pipeline::Job::Send(job)).is_none());
                    if pool.available().is_multiple_of(8) {
                        pool.flush();
                    }
                }
                Ok(Outbound::Queued { init }) => {
                    self.first_until = Some(now + FIRST_PACKET_POLL);
                    if let Some(init) = init {
                        self.handshake(secure, manager, init).await;
                    }
                }
                Ok(Outbound::AwaitingAdjacency) => {
                    self.first_until = Some(now + FIRST_PACKET_POLL);
                }
                Ok(Outbound::Direct | Outbound::Unmanaged) => {
                    self.drop_packet("local_egress_on_mesh_interface")
                }
                Ok(Outbound::Denied(_)) => self.drop_packet("netmap_denied"),
                Err(error) => self.drop_admission("packet_admission", error),
            }
        }
        if self.first_until.is_some_and(|until| until > now) {
            for flushed in secure.flush(pool.available().min(32)) {
                match flushed {
                    Ok(Flushed::Send(job)) => {
                        assert!(pool.submit(secure::pipeline::Job::Send(job)).is_none());
                    }
                    Ok(Flushed::Init(init)) => self.handshake(secure, manager, init).await,
                    Err(error) => self.drop_admission("first_packet", error),
                }
            }
        } else {
            self.first_until = None;
        }
        self.flush_sends(manager, &mut outgoing).await;
        pool.flush();
        self.complete_crypto(
            secure,
            manager,
            pool,
            exit.as_deref_mut(),
            &mut diagnostics.externals,
        );
        // Path MTU errors for this Node's own oversized packets reach its TUN
        // like any delivered packet.
        let errors = secure.take_local_errors();
        if !errors.is_empty()
            && let Some(tun) = host.tun()
        {
            let count = errors.len();
            if meshway_dataplane::carrier::deliver_batch(tun, errors)
                .await
                .is_err()
            {
                for _ in 0..count {
                    self.drop_packet("too_big_delivery");
                }
            }
        }
        if let Some(exit) = exit {
            exit.drive(secure);
        }
        self.trace_held(secure, pool, incoming_done, local_done);
        self.scan_remaining = self
            .scan_remaining
            .saturating_sub(incoming_done + local_done)
            .min(self.local.len() + self.incoming.len() + self.control.len());
        if !pool.can_submit() {
            self.scan_remaining = 0;
        }
    }

    /// Send a new handshake INIT and keep polling its retry schedule.
    async fn handshake(
        &mut self,
        secure: &SecureNode,
        manager: &Manager,
        init: secure::PendingFrame,
    ) {
        let circuit = init.circuit();
        if self.attempts.len() < MAX_ATTEMPTS
            && !self
                .attempts
                .iter()
                .any(|attempt| attempt.circuit == circuit)
        {
            self.attempts.push(Attempt {
                circuit,
                wake_at: secure
                    .handshake_retry_at(circuit)
                    .ok()
                    .flatten()
                    .map_or(Instant::now() + Duration::from_secs(10), Into::into),
            });
        }
        if manager.send(init).await.is_err() {
            self.drop_packet("handshake_send");
        }
    }

    async fn flush_sends(&mut self, manager: &Manager, outgoing: &mut Vec<secure::PendingFrame>) {
        let mut data = Vec::new();
        for frame in outgoing.drain(..) {
            if frame.is_data() {
                data.push(frame);
            } else if manager.send(frame).await.is_err() {
                self.drop_packet("cipher_send");
            }
        }
        if !data.is_empty() {
            let count = data.len();
            if manager.send_batch(data).await.is_err() {
                for _ in 0..count {
                    self.drop_packet("batch_send");
                }
            }
        }
    }

    fn complete_crypto(
        &mut self,
        secure: &mut SecureNode,
        manager: &Manager,
        pool: &mut secure::pipeline::Pool,
        mut exit: Option<&mut crate::host::ExitAttachment>,
        echoes: &mut super::diagnostics::external::Externals,
    ) {
        let mut outgoing = Vec::new();
        let mut deliveries = Vec::new();
        let mut send_credits = Vec::new();
        let mut delivery_credits = Vec::new();
        for output in pool.take_ready() {
            let (output, credit) = output.into_parts();
            let result = match output {
                Ok(secure::pipeline::Output::Send(frame)) => {
                    if let Ok(Some(circuit)) = secure.circuit_info(frame.circuit()) {
                        self.trace_packet(&frame, &circuit);
                    }
                    Ok(Received::Forward(frame))
                }
                Ok(secure::pipeline::Output::Receive(opened)) => secure.commit_receive(opened),
                Err(error) => Err(error),
            };
            match result {
                Ok(Received::Forward(frame)) => {
                    outgoing.push(frame);
                    send_credits.push(credit);
                }
                Ok(Received::Packet(packet)) if packet.hop() == meshway_dataplane::Hop::Deliver => {
                    // A `meshway ping` answer ends here, never in the TUN.
                    if echoes.answer(&packet) {
                        drop(credit);
                        continue;
                    }
                    deliveries.push(packet);
                    delivery_credits.push(credit);
                }
                Ok(Received::Packet(packet)) if packet.hop() == meshway_dataplane::Hop::Exit => {
                    if let Some(exit) = exit.as_deref_mut() {
                        exit.packet(secure, packet);
                    } else if let Some(flow) = packet.exit_flow() {
                        secure.fail_exit_projection(&flow);
                    }
                    // The exit FIFO has its own 128-packet bound; it retains the
                    // original wrapper. Release this crypto reservation now.
                    drop(credit);
                }
                Ok(Received::Keepalive { .. }) => drop(credit),
                Err(error) => self.drop_admission("crypto_admission", error),
                _ => self.drop_packet("crypto_output"),
            }
        }
        if !outgoing.is_empty() {
            let count = outgoing.len();
            // Freeze actual channel scopes before yielding to the output driver.
            let send = manager.send_batch(outgoing);
            self.sends
                .push(send_credits, async move { (send.await.map(|_| ()), count) });
        }
        if !deliveries.is_empty() {
            let count = deliveries.len();
            self.deliveries
                .push(delivery_credits, deliveries, Delivered { count });
        }
    }
}
