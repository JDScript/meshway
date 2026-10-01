//! Lease-fenced STUN observations from the carrier's existing receive dispatcher.
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

use chrono::{DateTime, Utc};
use futures_util::{StreamExt, stream};
use meshway_dataplane::{Runtime, UdpDiscoveryOutcome, UdpSocketInfo};
use meshway_protocol::{
    EndpointDiscoverySource, EndpointDiscoveryState as State, MAX_DISCOVERY_SOCKETS,
    StunDiscoveryOutcome, StunEndpointObservation, UdpDiscoveryResourceKind, UdpSocketDiscovery,
};
use tokio::{
    sync::watch,
    time::{self, Instant},
};

use super::{Observation, Phase, controller_candidate};

/// Retained across socket generations, including expiry/withdrawal. Wall-clock
/// regression suppresses new observations rather than manufacturing timestamps
/// or extending the previous monotonic/UTC expiry.
#[derive(Default)]
pub(super) struct TimestampGate(Mutex<Option<DateTime<Utc>>>);

impl TimestampGate {
    fn observe(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let mut watermark = self.0.lock().expect("discovery timestamp gate");
        if watermark.is_some_and(|previous| now <= previous) {
            return None;
        }
        *watermark = Some(now);
        Some(now)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Source {
    pub revision: u32,
    pub digest: String,
}

#[derive(Clone)]
pub(super) struct Publication {
    pub source: Source,
    pub state: State,
    pub sockets: Vec<UdpSocketDiscovery>,
    expiries: Vec<(u32, String, Instant)>,
}

impl Publication {
    pub fn new(source: Source, state: State) -> Self {
        Self {
            source,
            state,
            sockets: Vec::new(),
            expiries: Vec::new(),
        }
    }

    pub fn prune(&mut self) {
        let now = Instant::now();
        self.expiries.retain(|(id, _, expiry)| {
            *expiry > now && self.sockets.iter().any(|socket| socket.socket_id == *id)
        });
        for socket in &mut self.sockets {
            socket.observations.retain(|observation| {
                self.expiries.iter().any(|(id, observer, _)| {
                    *id == socket.socket_id && observer == &observation.observer
                })
            });
        }
        if matches!(self.state, State::Observed | State::Probing) {
            self.state = if self
                .sockets
                .iter()
                .flat_map(|s| &s.observations)
                .any(|o| o.outcome == StunDiscoveryOutcome::Mapped)
            {
                State::Observed
            } else {
                State::Probing
            };
        }
    }

    pub fn earliest_expiry(&self) -> Option<Instant> {
        self.expiries.iter().map(|(_, _, expiry)| *expiry).min()
    }
}

fn describe(
    socket: &UdpSocketInfo,
    intent: &meshway_protocol::NodeConnections,
    generation: u32,
) -> Option<UdpSocketDiscovery> {
    let (resource_kind, resource_id) = match (&socket.listener_id, &socket.candidate_id) {
        (Some(id), None) if intent.listeners.iter().any(|l| l.id == *id) => {
            (UdpDiscoveryResourceKind::Listener, id.clone())
        }
        (None, Some(label)) => (
            UdpDiscoveryResourceKind::OutgoingCandidate,
            controller_candidate(intent, label)?.id.clone(),
        ),
        _ => return None,
    };
    Some(UdpSocketDiscovery {
        resource_kind,
        resource_id,
        generation,
        // HTTP IDs are positive ordinals; core's dispatcher ordinals are zero-based.
        socket_id: socket.socket_id.checked_add(1)?,
        bound_address: socket.local_addr.to_string(),
        source: EndpointDiscoverySource::ManagedUdpSocketStun,
        observations: Vec::new(),
    })
}

/// The source is fixed for a generation. Validation retires it on source change.
/// Equivalent deadline renewal never changes stored observation expiry.
pub(super) struct Carrier {
    pub runtime: Weak<Runtime>,
    pub intent: meshway_protocol::NodeConnections,
    pub generation: u32,
    pub source: Source,
    pub timestamps: Arc<TimestampGate>,
}

pub(super) async fn run(
    carrier: Carrier,
    servers: Vec<std::net::SocketAddr>,
    mut deadline: watch::Receiver<Option<Instant>>,
    mut observed: watch::Receiver<Observation>,
    publication: watch::Sender<Publication>,
) {
    let Carrier {
        runtime,
        intent,
        generation,
        source,
        timestamps,
    } = carrier;
    let mut queue = VecDeque::new();
    let mut value = Publication::new(source.clone(), State::Probing);
    loop {
        let Some(until) = (*deadline.borrow_and_update()).filter(|d| *d > Instant::now()) else {
            break;
        };
        if observed.borrow_and_update().state != Phase::Applied {
            break;
        }
        let Some(runtime) = runtime.upgrade() else {
            break;
        };
        let sockets = runtime.udp_sockets();
        if sockets.len() > MAX_DISCOVERY_SOCKETS {
            value = Publication::new(source.clone(), State::Error);
            queue.clear();
        } else {
            let described: Option<Vec<_>> = sockets
                .iter()
                .map(|s| describe(s, &intent, generation))
                .collect();
            if let Some(described) = described {
                for mut socket in described {
                    if let Some(old) = value
                        .sockets
                        .iter()
                        .find(|s| s.socket_id == socket.socket_id)
                    {
                        socket.observations = old.observations.clone();
                    }
                    // Actual live inventory, including unmatched-family unknowns.
                    if let Some(old) = value
                        .sockets
                        .iter_mut()
                        .find(|s| s.socket_id == socket.socket_id)
                    {
                        *old = socket;
                    } else {
                        value.sockets.push(socket);
                    }
                }
                value.sockets.retain(|s| {
                    sockets
                        .iter()
                        .any(|live| live.socket_id.checked_add(1) == Some(s.socket_id))
                });
                let eligible: Vec<_> = sockets
                    .iter()
                    .flat_map(|socket| {
                        servers
                            .iter()
                            .filter(move |server| socket.local_addr.is_ipv4() == server.is_ipv4())
                            .map(move |server| (socket.socket_id, *server))
                    })
                    .collect();
                queue.retain(|job| eligible.contains(job));
                for job in eligible {
                    if !queue.contains(&job) {
                        queue.push_back(job);
                    }
                }
                if value.state == State::Error {
                    value.state = State::Probing;
                }
            } else {
                value = Publication::new(source.clone(), State::Error);
                queue.clear();
            }
        }
        value.prune();
        publication.send_replace(value.clone());
        let round_end = (Instant::now() + Duration::from_secs(10)).min(until);
        let count = queue.len();
        // The core deliberately permits one transaction per carrier socket.
        // Serialize its observers while allowing four sockets globally.
        let slots: BTreeMap<_, _> = sockets
            .iter()
            .map(|socket| (socket.socket_id, Arc::new(tokio::sync::Semaphore::new(1))))
            .collect();
        // Rotate when launched, so a slow/unresponsive observer cannot starve
        // later sockets when the bounded round is cancelled at its deadline.
        let changed = {
            let jobs = stream::iter(0..count)
                .map(|_| {
                    let (id, server) = queue.pop_front().expect("bounded queue");
                    queue.push_back((id, server));
                    let runtime = runtime.clone();
                    let slot = slots[&id].clone();
                    let timestamps = timestamps.clone();
                    async move {
                        let _permit = slot.acquire_owned().await.expect("round-owned semaphore");
                        let started = Instant::now();
                        let utc = timestamps.observe(Utc::now())?;
                        let result = runtime
                            .discover_udp(id, server, Duration::from_millis(1000))
                            .await;
                        Some((id, server, started, utc, result))
                    }
                })
                .buffer_unordered(4);
            tokio::pin!(jobs);
            loop {
                tokio::select! {
                    biased;
                    _ = time::sleep_until(round_end) => break false,
                    result = deadline.changed() => {
                        if result.is_err() { return; }
                        if !(*deadline.borrow_and_update()).is_some_and(|d| d > Instant::now()) { break true; }
                        // Equivalent renewal preserves this round's original expiry.
                    },
                    result = observed.changed() => {
                        if result.is_err() { return; }
                        if observed.borrow_and_update().state != Phase::Applied { break true; }
                        // Adjacency probe results do not retire the carrier socket.
                    },
                    result = jobs.next() => {
                    let Some(result) = result else { break false };
                    let Some((id, server, started, utc, result)) = result else { continue; };
                        let expiry = (started + Duration::from_secs(30)).min(until);
                        if expiry <= Instant::now() { continue; }
                        let Ok(result) = result else { continue; };
                        let outcome = match result.outcome {
                            UdpDiscoveryOutcome::Mapped => StunDiscoveryOutcome::Mapped,
                            UdpDiscoveryOutcome::Timeout => StunDiscoveryOutcome::Timeout,
                            UdpDiscoveryOutcome::InvalidResponse => StunDiscoveryOutcome::InvalidResponse,
                            UdpDiscoveryOutcome::IoError => StunDiscoveryOutcome::IoError,
                            UdpDiscoveryOutcome::StunError => StunDiscoveryOutcome::StunError,
                        };
                        let Some(public_id) = id.checked_add(1) else { continue; };
                        if let Some(socket) = value.sockets.iter_mut().find(|s| s.socket_id == public_id) {
                            let observer = server.to_string();
                            socket.observations.retain(|o| o.observer != observer);
                            socket.observations.push(StunEndpointObservation {
                                observer: observer.clone(), outcome,
                                mapped_endpoint: if outcome == StunDiscoveryOutcome::Mapped {
                                    result.mapped_endpoint.map(|address| address.to_string())
                                } else { None },
                                rtt_ms: result.round_trip.and_then(|rtt| u32::try_from(rtt.as_millis()).ok()),
                                observed_at: utc,
                                expires_at: absolute_expiry(utc, started, expiry),
                            });
                            value.expiries.retain(|(socket, old, _)| *socket != public_id || *old != observer);
                            value.expiries.push((public_id, observer, expiry));
                            value.prune();
                            publication.send_replace(value.clone());
                        }
                    }
                }
            }
        };
        if changed {
            continue;
        }
        tokio::select! {
            _ = time::sleep_until(round_end) => {},
            _ = deadline.changed() => {},
            _ = observed.changed() => {},
        }
    }
    publication.send_replace(Publication::new(source, State::Withdrawn));
}

fn absolute_expiry(utc: DateTime<Utc>, started: Instant, expiry: Instant) -> DateTime<Utc> {
    utc + chrono::Duration::from_std(expiry.saturating_duration_since(started))
        .expect("bounded TTL")
}

#[cfg(test)]
mod tests;
