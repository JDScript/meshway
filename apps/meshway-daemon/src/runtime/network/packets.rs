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

struct Queued {
    packet: Packet,
    deadline: Instant,
}
struct Incoming {
    carrier: Carrier,
    link: meshway_connectivity::LinkFact,
    received_at: std::time::Instant,
    bytes: Vec<u8>,
    deadline: Instant,
    guard: meshway_control::ExecutionSnapshot,
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

#[derive(Default)]
pub(super) struct Packets {
    local: VecDeque<Queued>,
    incoming: VecDeque<Incoming>,
    intents: Vec<Intent>,
    attempts: Vec<Attempt>,
    traced_circuits: BTreeMap<[u8; 32], Instant>,
    pub dropped: u64,
    carrier_wait_logged: Option<Instant>,
}

impl Packets {
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
        let absolute = |ms: u64| now + Duration::from_millis(ms.saturating_sub(now_ms));
        let mut next = self
            .local
            .iter()
            .map(|item| item.deadline)
            .chain(self.incoming.iter().map(|item| item.deadline))
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
        self.dropped = self.dropped.saturating_add(self.incoming.len() as u64);
        self.incoming.clear();
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
        bytes: Vec<u8>,
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
        // Hints only decide bounded scheduling. receive() authenticates every
        // byte and the actual adjacent carrier before any output or flow change.
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
        if !known
            || bytes.len() > secure::MAX_FRAME
            || self.local.len() + self.incoming.len() >= MAX_PACKETS
            || self.bytes() + bytes.len() > MAX_BYTES
        {
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
        self.incoming.push_back(Incoming {
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
        host: &meshway_underlay::Host,
        target: &Target,
        gateway: &mpsc::Sender<Request>,
        diagnostics: &mut super::diagnostics::Diagnostics,
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
        let Ok(carriers) = manager.carriers() else {
            return;
        };
        for _ in 0..self.incoming.len() {
            let Some(item) = self.incoming.pop_front() else {
                break;
            };
            if item.deadline <= now || item.guard.ensure_current().is_err() {
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
                self.incoming.push_back(item);
                continue;
            }
            if meshway_logging::profile::enabled() {
                static WAIT: meshway_logging::profile::Metric =
                    meshway_logging::profile::Metric::new("queue.manager_to_secure");
                WAIT.elapsed(item.received_at.elapsed());
            }
            match secure.receive(&item.carrier, item.bytes) {
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
                        manager.send(frame).await.map_err(|_| ())
                    };
                    if sent.is_err() {
                        self.drop_packet("cipher_send");
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
                    if let Ok(channel) = super::diagnostics::channel(&item.link) {
                        diagnostics.owner.reply(
                            circuit,
                            measurement_id,
                            challenge,
                            channel,
                            item.received_at,
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
                    if let Some(tun) = host.tun() {
                        if meshway_dataplane::carrier::deliver(tun, packet)
                            .await
                            .is_err()
                        {
                            self.drop_packet("tun_delivery");
                        } else if let Some(demand) = reply_demand
                            && snapshot.ensure_current().is_ok()
                            && let Ok(delivered_ms) = snapshot.now_ms()
                            && let Some(intent) = self.intents.iter_mut().find(|intent| {
                                intent.request.request_id == demand
                                    && intent.expires_at_ms > delivered_ms
                                    && intent.last_packet.elapsed() < Duration::from_secs(60)
                            })
                        {
                            // Only authenticated, admitted replies actually written
                            // to this origin's TUN sustain its existing renewal owner.
                            // Hints, forwarding, failed output and expired/cancelled
                            // owners cannot create or revive an intent.
                            intent.last_packet = Instant::now();
                        }
                    }
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
        for _ in 0..self.local.len() {
            let Some(item) = self.local.pop_front() else {
                break;
            };
            if item.deadline <= now {
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
                match secure.send(circuit.id, item.packet) {
                    Ok(frame) => {
                        self.trace_packet(&frame, &circuit);
                        if manager.send(frame).await.is_err() {
                            self.drop_packet("packet_send");
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
