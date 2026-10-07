//! Bounded scheduling of original packets and control intents. There is no
//! second flow table: every local packet consults SecureNode's unique owner.
use super::{Operation, Reply, Request, Target};
use meshway_configuration::execution::{
    BusinessTarget, CircuitAuthorization, DemandDecision, ExecutionAddress,
};
use meshway_connectivity::{Carrier, Manager};
use meshway_dataplane::{
    Packet,
    secure::{self, CircuitState, Received, SecureNode},
};
use meshway_model::{NodeId, RequestId};
use meshway_protocol::{DemandRequest, RenewDemandRequest};
use std::{
    collections::{BTreeMap, VecDeque},
    net::IpAddr,
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

const MAX_PACKETS: usize = 128;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_INTENTS: usize = 64;
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
    guard: meshway_authority::ExecutionSnapshot,
}
struct Intent {
    request: DemandRequest,
    last_packet: Instant,
    retry: Instant,
    pending: Option<oneshot::Receiver<Reply>>,
    created: bool,
    renewal: Option<RenewDemandRequest>,
    expires_at_ms: u64,
    renewed_until: u64,
}
struct Attempt {
    demand: RequestId,
    path: Vec<NodeId>,
    circuit: [u8; 32],
    wake_at: Instant,
}

struct Delivered {
    count: usize,
    replies: Vec<RequestId>,
    snapshot: meshway_authority::ExecutionSnapshot,
}

#[derive(Default)]
pub(super) struct Packets {
    local: VecDeque<Queued>,
    incoming: VecDeque<Incoming>,
    control: VecDeque<Incoming>,
    intents: Vec<Intent>,
    attempts: Vec<Attempt>,
    traced_circuits: BTreeMap<[u8; 32], Instant>,
    pub dropped: u64,
    carrier_wait_logged: Option<Instant>,
    scan_remaining: usize,
    sends:
        meshway_dataplane::carrier::SendPending<(Result<(), meshway_connectivity::Error>, usize)>,
    deliveries: meshway_dataplane::carrier::TunPending<Delivered>,
}

impl Packets {
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
    pub async fn sent(&mut self, host: &meshway_underlay::Host) {
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
                } else if let Ok(now_ms) = delivered.snapshot.now_ms() {
                    for demand in delivered.replies {
                        if let Some(intent) = self.intents.iter_mut().find(|intent| {
                            intent.request.request_id == demand && intent.expires_at_ms > now_ms
                                && intent.last_packet.elapsed() < Duration::from_secs(60)
                        }) { intent.last_packet = Instant::now(); }
                    }
                }
            }
        }
    }

    fn trace_packet(&mut self, frame: &secure::PendingFrame, circuit: &secure::CircuitInfo) {
        const TARGET: &str = "meshway_daemon::runtime::network::circuit_packet";
        if !tracing::enabled!(target: TARGET, tracing::Level::DEBUG) {
            return;
        }
        // This observation is emitted only for admitted OS business packets.
        // Diagnostic observations have their own typed measurement events and
        // cannot be represented as a business demand, even in debug output.
        let CircuitAuthorization::Business(demand) = circuit.authorization else {
            return;
        };
        let now = Instant::now();
        let id = frame.circuit();
        if self
            .traced_circuits
            .get(&id)
            .is_some_and(|previous| now.duration_since(*previous) < Duration::from_secs(1))
        {
            return;
        }
        if self.traced_circuits.len() >= MAX_INTENTS {
            self.traced_circuits
                .retain(|_, previous| now.duration_since(*previous) < Duration::from_secs(5));
            if !self.traced_circuits.contains_key(&id) && self.traced_circuits.len() >= MAX_INTENTS
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
        tracing::debug!(target: TARGET, %cid, %demand, %path_json, observed_at_ms,
            "Mesh circuit packet ready");
    }
    pub fn wake_at(&self, target: &Target) -> Option<Instant> {
        let snapshot = target.identity.as_ref()?.snapshot();
        let cfg = snapshot.configuration().ok()?;
        let now_ms = snapshot.now_ms().ok()?;
        let now = Instant::now();
        if self.scan_remaining != 0 {
            return Some(now);
        }
        let absolute = |ms: u64| now + Duration::from_millis(ms.saturating_sub(now_ms));
        let mut next = self
            .local
            .iter()
            .map(|item| item.deadline)
            .chain(self.incoming.iter().map(|item| item.deadline))
            .chain(self.control.iter().map(|item| item.deadline))
            .chain(self.attempts.iter().map(|attempt| attempt.wake_at))
            .min();
        for intent in &self.intents {
            let mut at =
                (intent.last_packet + Duration::from_secs(60)).min(absolute(intent.expires_at_ms));
            if intent.pending.is_some() {
                // Only an outstanding gateway reply needs polling. Kept idle
                // intents wait for their actual lease/idle deadline or handoff.
                at = at.min(now + Duration::from_millis(100));
            } else if !intent.created || intent.renewal.is_some() {
                at = at.min(intent.retry.max(now + Duration::from_millis(100)));
            } else if target.instance_expires_at_ms > intent.expires_at_ms
                && intent.expires_at_ms >= intent.renewed_until
                && cfg.permits.contains_key(&intent.request.request_id)
            {
                at = at.min(
                    absolute(intent.expires_at_ms.saturating_sub(20_000))
                        .max(intent.retry)
                        .max(now + Duration::from_millis(100)),
                );
            }
            next = Some(next.map_or(at, |old| old.min(at)));
        }
        next
    }
    pub fn clear_incoming(&mut self) {
        self.dropped = self.dropped.saturating_add(
            (self.incoming.len()
                + self.control.len()
                + self.sends.occupancy().0
                + self.deliveries.occupancy().0) as u64,
        );
        self.incoming.clear();
        self.control.clear();
        self.sends = Default::default();
        self.deliveries = Default::default();
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

    fn drop_admission(
        &mut self,
        reason: &'static str,
        error: secure::Error,
        demand: Option<RequestId>,
    ) {
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
                _ => "other",
            },
        );
        if self.dropped.is_power_of_two() {
            tracing::debug!(
                reason,
                ?error,
                ?demand,
                demand_untrusted = reason == "cipher_admission",
                dropped = self.dropped,
                "Packet discarded"
            );
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
        target: &Target,
        secure: &SecureNode,
    ) {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("host.incoming");
        let _profile = PROFILE.enter();
        let Some(identity) = &target.identity else {
            return;
        };
        let Ok(cfg) = identity.snapshot().configuration() else {
            return;
        };
        let Ok(now_ms) = identity.snapshot().now_ms() else {
            return;
        };
        // Hints only decide bounded scheduling. receive() authenticates packet
        // content, this verifier's full path prefix and the actual carrier;
        // downstream MAC columns are checked by their respective verifiers.
        let Ok(hint) = secure.incoming_hint(&bytes) else {
            self.drop_packet("cipher_header");
            return;
        };
        let known = match hint.authorization {
            CircuitAuthorization::Business(id) => cfg.permits.get(&id).is_some_and(|permit| {
                permit.expires_at_ms > now_ms
                    && permit.paths.iter().any(|path| path.nodes == hint.path)
            }),
            CircuitAuthorization::Diagnostic(id) => {
                cfg.diagnostics.get(&id).is_some_and(|permit| {
                    permit.expires_at_ms > now_ms
                        && permit.paths.iter().any(|path| path.nodes == hint.path)
                })
            }
        };
        if matches!(hint.authorization, CircuitAuthorization::Diagnostic(_)) {
            tracing::debug!(authorization_untrusted = ?hint.authorization, local = %cfg.node_id,
                publication = %identity.snapshot().reference().execution_publication,
                known, "Diagnostic carrier frame received for scheduling");
        }
        let control = bytes.get(8) != Some(&3);
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
            guard: identity.snapshot().clone(),
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn drive(
        &mut self,
        secure: &mut SecureNode,
        manager: &Manager,
        carriers: Option<&BTreeMap<NodeId, Carrier>>,
        host: &meshway_underlay::Host,
        target: &Target,
        gateway: &mpsc::Sender<Request>,
        diagnostics: &mut super::diagnostics::Diagnostics,
        pool: &mut secure::pipeline::Pool,
    ) {
        let Some(identity) = &target.identity else {
            return;
        };
        let snapshot = identity.snapshot();
        let Ok(cfg) = snapshot.configuration() else {
            return;
        };
        let Ok(now_ms) = snapshot.now_ms() else {
            return;
        };
        let now = Instant::now();
        for intent in &mut self.intents {
            if let Some(permit) = cfg.permits.get(&intent.request.request_id) {
                intent.expires_at_ms = permit.expires_at_ms;
            }
        }
        self.intents.retain(|intent| {
            intent.expires_at_ms > now_ms
                && now.duration_since(intent.last_packet) < Duration::from_secs(60)
        });
        let Some(carriers) = carriers else {
            return;
        };
        self.complete_crypto(secure, manager, snapshot, pool);
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
            if item.bytes.get(8) == Some(&3)
                && (!pool.can_submit_capacity(item.bytes.capacity())
                    || quantum.elapsed() >= Duration::from_micros(200))
            {
                self.incoming.push_front(item);
                break;
            }
            if item.deadline <= Instant::now() || item.guard.ensure_current().is_err() {
                self.drop_packet("cipher_deadline");
                continue;
            }
            let hint = secure.incoming_hint(&item.bytes).ok();
            let waiting = hint.as_ref().is_some_and(|hint| {
                let Some(position) = hint.path.iter().position(|node| *node == cfg.node_id) else {
                    return false;
                };
                let previous = if item.carrier.dialer == cfg.node_id {
                    item.carrier.listener
                } else {
                    item.carrier.dialer
                };
                let next =
                    if position.checked_sub(1).and_then(|p| hint.path.get(p)) == Some(&previous) {
                        hint.path.get(position + 1)
                    } else {
                        position.checked_sub(1).and_then(|p| hint.path.get(p))
                    };
                next.is_some_and(|node| !carriers.contains_key(node))
            });
            if waiting {
                if self
                    .carrier_wait_logged
                    .is_none_or(|at| at.elapsed() >= Duration::from_secs(1))
                {
                    self.carrier_wait_logged = Some(Instant::now());
                    tracing::debug!(authorization_untrusted = ?hint.as_ref().map(|hint| hint.authorization), path_untrusted = ?hint.as_ref().map(|hint| &hint.path), local = %cfg.node_id,
                        publication = %snapshot.reference().execution_publication,
                        available = ?carriers.keys().collect::<Vec<_>>(), "Waiting for authenticated next-hop carrier before secure receive");
                }
                if item.bytes.get(8) == Some(&3) {
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
            let result = if item.bytes.get(8) == Some(&3) {
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
        let mut reply_demands = Vec::new();
        for (link, received_at, hint, result) in ready {
            let result = result.expect("synchronous control result");
            match result {
                Ok(Received::Forward(frame))
                | Ok(Received::Established {
                    response: Some(frame),
                    ..
                }) => {
                    let sent = if hint.as_ref().is_some_and(|hint| {
                        matches!(hint.authorization, CircuitAuthorization::Diagnostic(_))
                    }) {
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
                Ok(Received::Established { response: None, .. }) => {}
                Ok(Received::ProbeRequest {
                    circuit,
                    measurement_id,
                    response,
                }) => {
                    tracing::debug!(?circuit, %measurement_id, publication = %snapshot.reference().execution_publication, "Authenticated diagnostic request received");
                    if diagnostics.forward(manager, response).is_err() {
                        self.drop_packet("probe_reply_send");
                    }
                }
                Ok(Received::ProbeReply {
                    circuit,
                    measurement_id,
                    challenge,
                }) => {
                    tracing::debug!(?circuit, %measurement_id, publication = %snapshot.reference().execution_publication, "Authenticated diagnostic reply received");
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
                    if packet.hop() != meshway_dataplane::Hop::Deliver {
                        self.drop_packet("unsupported_exit");
                        continue;
                    }
                    let reply_demand = (packet.route().direction
                        == meshway_dataplane::Direction::Reply
                        && packet.route().origin == cfg.node_id)
                        .then_some(packet.route().demand);
                    deliveries.push(packet);
                    reply_demands.extend(reply_demand);
                }
                Err(error) => self.drop_admission(
                    "cipher_admission",
                    error,
                    hint.and_then(|hint| match hint.authorization {
                        CircuitAuthorization::Business(id) => Some(id),
                        CircuitAuthorization::Diagnostic(_) => None,
                    }),
                ),
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
            } else if snapshot.ensure_current().is_ok()
                && let Ok(delivered_ms) = snapshot.now_ms()
            {
                for demand in reply_demands {
                    if let Some(intent) = self.intents.iter_mut().find(|intent| {
                        intent.request.request_id == demand
                            && intent.expires_at_ms > delivered_ms
                            && intent.last_packet.elapsed() < Duration::from_secs(60)
                    }) {
                        intent.last_packet = Instant::now();
                    }
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
            let facts = item.packet.facts();
            let source_matches = match facts.traffic.source {
                IpAddr::V4(address) => cfg.addresses.ipv4 == Some(address),
                IpAddr::V6(address) => cfg.addresses.ipv6 == Some(address),
            };
            if !source_matches {
                self.drop_packet("local_source");
                continue;
            }
            let route = match secure.local_flow(&item.packet) {
                Err(_) => {
                    self.drop_packet("flow_role");
                    continue;
                }
                Ok(Some(flow)) => {
                    if let Some(intent) = self
                        .intents
                        .iter_mut()
                        .find(|intent| intent.request.request_id == flow.demand)
                    {
                        intent.last_packet = now;
                    }
                    Some((flow.demand, flow.path))
                }
                Ok(None) => {
                    let business = BusinessTarget::Ip {
                        address: ExecutionAddress(facts.traffic.destination),
                    };
                    let index = self.intents.iter().position(|intent| {
                        intent.request.target == business
                            && intent.request.protocol == facts.traffic.protocol
                            && intent.request.port == facts.traffic.destination_port
                            && intent.request.profile == cfg.dns.profile
                    });
                    let index = match index {
                        Some(index) => index,
                        None if self.intents.len() < MAX_INTENTS => {
                            let Some(request_id) = request_id() else {
                                self.drop_packet("request_id");
                                continue;
                            };
                            self.intents.push(Intent {
                                request: DemandRequest {
                                    instance: target.ticket.instance().clone(),
                                    request_id,
                                    target: business,
                                    protocol: facts.traffic.protocol,
                                    port: facts.traffic.destination_port,
                                    profile: cfg.dns.profile.clone(),
                                    duration_seconds: 120,
                                },
                                last_packet: now,
                                retry: now,
                                pending: None,
                                created: false,
                                renewal: None,
                                expires_at_ms: target
                                    .instance_expires_at_ms
                                    .min(now_ms.saturating_add(120_000)),
                                renewed_until: 0,
                            });
                            self.intents.len() - 1
                        }
                        None => {
                            self.drop_packet("intent_capacity");
                            continue;
                        }
                    };
                    let intent = &mut self.intents[index];
                    intent.last_packet = now;
                    let id = intent.request.request_id;
                    if cfg.decisions.get(&id).is_some_and(|decision| {
                        !matches!(decision, DemandDecision::Permitted { .. })
                    }) {
                        self.drop_packet("demand_rejected");
                        continue;
                    }
                    cfg.permits
                        .get(&id)
                        .and_then(|permit| permit.paths.first())
                        .map(|path| (id, path.nodes.clone()))
                }
            };
            let Some((demand, path)) = route else {
                self.local.push_back(item);
                continue;
            };
            let ready = match secure.local_circuit(demand, &path, &item.packet) {
                Ok(ready) => ready,
                Err(error) => {
                    self.drop_admission("circuit_selection", error, Some(demand));
                    continue;
                }
            };
            if let Some(circuit) = ready {
                let next = if path.first() == Some(&cfg.node_id) {
                    path.get(1)
                } else {
                    path.len().checked_sub(2).and_then(|index| path.get(index))
                };
                if next.is_none_or(|next| !carriers.contains_key(next)) {
                    self.local.push_back(item);
                    continue;
                }
                match secure.prepare_send_packet(circuit.id, item.packet) {
                    Ok(job) => {
                        assert!(pool.submit(secure::pipeline::Job::Send(job)).is_none());
                        if pool.available().is_multiple_of(8) {
                            pool.flush();
                        }
                    }
                    Err(error) => self.drop_admission("packet_admission", error, Some(demand)),
                }
            } else {
                if path.first() == Some(&cfg.node_id)
                    && path.get(1).is_some_and(|next| carriers.contains_key(next))
                    && self.attempts.len() < MAX_INTENTS
                    && !self
                        .attempts
                        .iter()
                        .any(|attempt| attempt.demand == demand && attempt.path == path)
                    && let Ok(frame) = secure.start(demand, path.clone())
                {
                    let circuit = frame.circuit();
                    self.attempts.push(Attempt {
                        demand,
                        path,
                        circuit,
                        wake_at: secure
                            .handshake_retry_at(circuit)
                            .ok()
                            .flatten()
                            .map_or(now + Duration::from_secs(10), Into::into),
                    });
                    if manager.send(frame).await.is_err() {
                        self.drop_packet("handshake_send");
                    }
                }
                self.local.push_back(item);
            }
        }
        self.flush_sends(manager, &mut outgoing).await;
        pool.flush();
        self.complete_crypto(secure, manager, snapshot, pool);
        self.scan_remaining = self
            .scan_remaining
            .saturating_sub(incoming_done + local_done)
            .min(self.local.len() + self.incoming.len() + self.control.len());
        if !pool.can_submit() {
            self.scan_remaining = 0;
        }
        for intent in &mut self.intents {
            if let Some(pending) = &mut intent.pending {
                match pending.try_recv() {
                    Ok(reply) => {
                        intent.pending = None;
                        if reply.result.is_ok() {
                            intent.created = true;
                            intent.renewal = None;
                            if let Some(renewal) = reply.renewal {
                                intent.renewed_until = renewal.expires_at_ms;
                            }
                        } else {
                            intent.renewal = reply.renewal;
                            intent.retry = now + Duration::from_secs(2);
                        }
                    }
                    Err(oneshot::error::TryRecvError::Closed) => {
                        intent.pending = None;
                        intent.retry = now + Duration::from_secs(2);
                    }
                    Err(oneshot::error::TryRecvError::Empty) => continue,
                }
            }
            if now < intent.retry {
                continue;
            }
            let operation = if !intent.created {
                Some(Operation::Create(intent.request.clone()))
            } else if let Some(renewal) = &intent.renewal {
                Some(Operation::Renew {
                    request: renewal.clone(),
                    prepared: true,
                })
            } else if let Some(permit) = cfg.permits.get(&intent.request.request_id)
                && permit.expires_at_ms <= now_ms.saturating_add(20_000)
                && target.instance_expires_at_ms > permit.expires_at_ms
                && permit.expires_at_ms >= intent.renewed_until
            {
                let renewal = RenewDemandRequest {
                    instance: intent.request.instance.clone(),
                    request_id: intent.request.request_id,
                    expires_at_ms: target.instance_expires_at_ms,
                };
                Some(Operation::Renew {
                    request: renewal,
                    prepared: false,
                })
            } else {
                None
            };
            if let Some(operation) = operation {
                let (reply, pending) = oneshot::channel();
                if gateway
                    .try_send(Request::Demand { operation, reply })
                    .is_ok()
                {
                    intent.pending = Some(pending);
                }
                intent.retry = now + Duration::from_secs(2);
            }
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
        snapshot: &meshway_authority::ExecutionSnapshot,
        pool: &mut secure::pipeline::Pool,
    ) {
        let mut outgoing = Vec::new();
        let mut deliveries = Vec::new();
        let mut replies = Vec::new();
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
                    if packet.route().direction == meshway_dataplane::Direction::Reply {
                        replies.push(packet.route().demand);
                    }
                    deliveries.push(packet);
                    delivery_credits.push(credit);
                }
                Err(error) => self.drop_admission("crypto_admission", error, None),
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
            self.deliveries.push(
                delivery_credits,
                deliveries,
                Delivered {
                    count,
                    replies,
                    snapshot: snapshot.clone(),
                },
            );
        }
    }
}

pub(super) fn request_id() -> Option<RequestId> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
    .parse()
    .ok()
}
