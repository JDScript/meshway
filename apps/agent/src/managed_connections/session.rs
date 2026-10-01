use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::{StreamExt, stream};
use meshway_dataplane::{
    PeerConnection, PrivateKey, Runtime, StreamContext, StreamRole, TransportKind,
};
use meshway_protocol::{
    ConnectionListenerObservation, ConnectionListenerState, ConnectionNeighborObservation,
    ConnectionNeighborState, ConnectionTransport, NodeConnections, OutgoingConnectionCandidate,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinSet,
    time::{self, Instant},
};

use super::{
    Failure, Observation, Phase, Reason, allowed_peers, candidate_label, controller_candidate,
    core_config,
    profiles::{self, Profiles},
    protocol_transport,
};

pub(super) struct Session {
    pub runtime: Arc<Runtime>,
    tasks: JoinSet<()>,
    outcomes: Outcomes,
    standby_candidates: BTreeSet<String>,
}

type Outcomes = Arc<Mutex<BTreeMap<String, Reason>>>;

impl Session {
    #[cfg(test)]
    pub async fn test_probe_observations(
        &self,
        intent: &NodeConnections,
    ) -> Vec<ConnectionNeighborObservation> {
        probe_peers(
            &self.runtime,
            intent,
            &self.outcomes,
            &self.standby_candidates,
            Instant::now(),
        )
        .await
    }

    pub async fn shutdown(&mut self) {
        self.tasks.abort_all();
        while self.tasks.join_next().await.is_some() {}
        self.runtime.shutdown().await;
    }

    #[cfg(test)]
    pub async fn start(
        intent: &NodeConnections,
        key: PrivateKey,
        profiles: Profiles,
        targets: &BTreeMap<String, Vec<SocketAddr>>,
    ) -> Result<Self, Failure> {
        Self::start_with_packet(intent, key, profiles, targets, None).await
    }

    pub async fn start_with_packet(
        intent: &NodeConnections,
        key: PrivateKey,
        profiles: Profiles,
        targets: &BTreeMap<String, Vec<SocketAddr>>,
        packet: Option<&crate::managed_dataplane::PacketPlan>,
    ) -> Result<Self, Failure> {
        let mut config = core_config(intent, key, targets)?;
        if let Some(plan) = packet {
            plan.augment(&mut config)
                .map_err(|_| Failure(Reason::InvalidConfiguration))?;
        }
        let active_native: BTreeSet<_> = config
            .outgoing
            .iter()
            .filter_map(|candidate| candidate.id.as_deref())
            .filter_map(|label| controller_candidate(intent, label))
            .map(|candidate| candidate.id.as_str())
            .collect();
        let standby_candidates = intent
            .outgoing
            .iter()
            .filter(|candidate| {
                candidate.transport != ConnectionTransport::Vless
                    && !active_native.contains(candidate.id.as_str())
            })
            .map(|candidate| candidate.id.clone())
            .collect();
        let mut listeners = Vec::new();
        for listener in intent
            .listeners
            .iter()
            .filter(|item| item.transport == ConnectionTransport::Vless)
        {
            let profile = listener
                .credential_ref
                .as_ref()
                .and_then(|id| profiles.servers.get(id))
                .ok_or(Failure(Reason::ProfileUnavailable))?
                .clone();
            let address = listener
                .bind
                .parse::<SocketAddr>()
                .map_err(|_| Failure(Reason::InvalidConfiguration))?;
            let socket = TcpListener::bind(address)
                .await
                .map_err(|_| Failure(Reason::BindFailed))?;
            listeners.push((
                socket,
                listener.id.clone(),
                allowed_peers(intent, &listener.id),
                profile,
            ));
        }
        // All external binds precede core's atomic native bind/start. Failure
        // drops those sockets; no partially installed listener survives.
        let runtime = Arc::new(
            Runtime::start(config)
                .await
                .map_err(|_| Failure(Reason::BindFailed))?,
        );
        let mut session = Self {
            runtime,
            tasks: JoinSet::new(),
            outcomes: Arc::default(),
            standby_candidates,
        };
        let handshakes = Arc::new(Semaphore::new(32));
        for (socket, id, peers, profile) in listeners {
            session.tasks.spawn(accept(
                session.runtime.clone(),
                socket,
                id,
                peers,
                profile,
                handshakes.clone(),
            ));
        }
        let priorities: Arc<BTreeMap<String, u16>> = Arc::new(
            intent
                .outgoing
                .iter()
                .flat_map(|item| {
                    (0..8).map(move |index| (candidate_label(&item.id, index), item.priority))
                })
                .collect(),
        );
        for candidate in intent
            .outgoing
            .iter()
            .filter(|item| item.transport == ConnectionTransport::Vless)
        {
            // Profiles and resolved targets were checked before any bind/start.
            let profile = profiles
                .clients
                .get(
                    candidate
                        .credential_ref
                        .as_ref()
                        .expect("validated profile reference"),
                )
                .expect("validated client profile")
                .clone();
            session.tasks.spawn(dial(
                session.runtime.clone(),
                candidate.clone(),
                targets[&candidate.id].clone(),
                profile,
                priorities.clone(),
                session.outcomes.clone(),
                handshakes.clone(),
            ));
        }
        Ok(session)
    }

    pub fn initial_observation(&self, intent: &NodeConnections) -> Observation {
        Observation {
            state: Phase::Applied,
            reason: Reason::None,
            listeners: intent
                .listeners
                .iter()
                .map(|item| ConnectionListenerObservation {
                    listener_id: item.id.clone(),
                    state: ConnectionListenerState::Bound,
                    reason: Reason::None,
                })
                .collect(),
            peers: waiting(intent),
        }
    }

    pub async fn run_with_packet(
        mut self,
        intent: NodeConnections,
        mut deadlines: watch::Receiver<Option<Instant>>,
        observed: watch::Sender<Observation>,
        packet: Option<(
            Arc<crate::managed_dataplane::tun::Device>,
            String,
            crate::managed_dataplane::PacketPlan,
        )>,
    ) {
        let probe_runtime = self.runtime.clone();
        let probe_observed = observed.clone();
        let outcomes = self.outcomes.clone();
        let standby_candidates = self.standby_candidates.clone();
        let probe = async move {
            let started = Instant::now();
            let mut tick = time::interval(Duration::from_secs(5));
            tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let peers = probe_peers(
                    &probe_runtime,
                    &intent,
                    &outcomes,
                    &standby_candidates,
                    started,
                )
                .await;
                probe_observed.send_modify(|value| value.peers = peers);
            }
        };
        tokio::pin!(probe);
        let packet_runtime = self.runtime.clone();
        let packet_deadline = deadlines.clone();
        let packet_io = async {
            match &packet {
                Some((device, name, plan)) => {
                    crate::managed_dataplane::packet::run(
                        device.clone(),
                        packet_runtime,
                        plan.clone(),
                        packet_deadline,
                        name,
                    )
                    .await
                }
                None => std::future::pending().await,
            }
        };
        tokio::pin!(packet_io);
        let reason = loop {
            let Some(deadline) = *deadlines.borrow_and_update() else {
                break Reason::ShuttingDown;
            };
            if Instant::now() >= deadline {
                break Reason::LeaseExpired;
            }
            tokio::select! {
                biased;
                _ = time::sleep_until(deadline) => break Reason::LeaseExpired,
                result = deadlines.changed() => if result.is_err() { break Reason::ShuttingDown; },
                _ = &mut packet_io => break Reason::InternalError,
                _ = &mut probe => break Reason::InternalError,
                _ = self.tasks.join_next(), if !self.tasks.is_empty() => break Reason::TransportFailed,
            }
        };
        observed.send_replace(Observation::empty(Phase::Withdrawn, reason));
        if let Some((_, name, plan)) = &packet {
            crate::managed_dataplane::tun::proof(
                name,
                plan,
                false,
                match reason {
                    Reason::LeaseExpired => "lease_expired",
                    Reason::ShuttingDown => "generation_withdrawn",
                    _ => "runtime_or_device_failure",
                },
            );
        }
        self.shutdown().await;
    }
}

fn waiting(intent: &NodeConnections) -> Vec<ConnectionNeighborObservation> {
    intent
        .outgoing
        .iter()
        .take(256)
        .map(|item| ConnectionNeighborObservation {
            peer_node_id: item.peer_node_id.clone(),
            transport: item.transport,
            initiator_node_id: intent.node_id.clone(),
            listener_id: Some(item.listener_id.clone()),
            candidate_id: Some(item.id.clone()),
            state: ConnectionNeighborState::Waiting,
            reason: Reason::None,
            probe_fresh: false,
            probe_rtt_ms: None,
        })
        .collect()
}

pub(super) fn describe(
    intent: &NodeConnections,
    channel: &PeerConnection,
) -> Option<ConnectionNeighborObservation> {
    let transport = protocol_transport(channel.transport);
    let (listener_id, candidate_id, initiator_node_id) = if channel.initiator {
        let candidate = controller_candidate(intent, channel.candidate_id.as_deref()?)?;
        if candidate.peer_node_id != channel.peer_node_id || candidate.transport != transport {
            return None;
        }
        (
            candidate.listener_id.clone(),
            Some(candidate.id.clone()),
            intent.node_id.clone(),
        )
    } else {
        let approval = intent.incoming.iter().find(|approval| {
            approval.peer_node_id == channel.peer_node_id
                && Some(&approval.listener_id) == channel.listener_id.as_ref()
                && approval.transport == transport
        })?;
        (
            approval.listener_id.clone(),
            None,
            channel.peer_node_id.clone(),
        )
    };
    Some(ConnectionNeighborObservation {
        peer_node_id: channel.peer_node_id.clone(),
        transport,
        initiator_node_id,
        listener_id: Some(listener_id),
        candidate_id,
        state: ConnectionNeighborState::Unreachable,
        reason: Reason::ProbeFailed,
        probe_fresh: false,
        probe_rtt_ms: None,
    })
}

async fn probe_peers(
    runtime: &Runtime,
    intent: &NodeConnections,
    outcomes: &Outcomes,
    standby_candidates: &BTreeSet<String>,
    started: Instant,
) -> Vec<ConnectionNeighborObservation> {
    let mut result: Vec<_> = stream::iter(runtime.peer_connections().into_iter().map(
        |channel| async move {
            let mut observation = describe(intent, &channel)?;
            if let Ok(proof) = runtime
                .probe_neighbor_observed(&channel.peer_node_id, Duration::from_secs(2))
                .await
            {
                // Metadata and nonce proof are captured atomically by core. A
                // replacement between listing and probing must never borrow the
                // earlier row's transport, initiator or local context labels.
                if let Some(proven) = describe(intent, &proof.connection) {
                    observation = proven;
                    observation.state = ConnectionNeighborState::Authenticated;
                    observation.reason = Reason::None;
                    observation.probe_fresh = true;
                    observation.probe_rtt_ms =
                        Some(proof.round_trip.as_millis().min(u32::MAX as u128) as u32);
                }
            }
            if let Some(candidate) = &observation.candidate_id {
                outcomes
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(candidate.clone(), observation.reason);
            }
            Some(observation)
        },
    ))
    .buffer_unordered(32)
    .filter_map(|value| async { value })
    .collect()
    .await;
    for mut candidate in waiting(intent) {
        if !result.iter().any(|item| {
            item.peer_node_id == candidate.peer_node_id
                && item.transport == candidate.transport
                && item.initiator_node_id == candidate.initiator_node_id
                && item.listener_id == candidate.listener_id
                && item.candidate_id == candidate.candidate_id
        }) && result.len() < 256
        {
            let configured = intent
                .outgoing
                .iter()
                .find(|item| Some(&item.id) == candidate.candidate_id.as_ref())
                .expect("own candidate");
            let standby = standby_candidates.contains(&configured.id)
                || runtime.peer_connections().iter().any(|current| {
                    current.peer_node_id == candidate.peer_node_id
                        && (!current.initiator
                            || current
                                .candidate_id
                                .as_ref()
                                .and_then(|id| controller_candidate(intent, id))
                                .is_some_and(|active| active.priority <= configured.priority))
                });
            let reason = outcomes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&configured.id)
                .copied()
                .filter(|reason| *reason != Reason::None)
                .or_else(|| {
                    (started.elapsed() >= Duration::from_secs(5)).then_some(Reason::TransportFailed)
                });
            if !standby && let Some(reason) = reason {
                candidate.state = ConnectionNeighborState::Unreachable;
                candidate.reason = reason;
            }
            result.push(candidate);
        }
    }
    result.sort_by(|a, b| {
        (
            &a.peer_node_id,
            a.transport,
            &a.initiator_node_id,
            &a.listener_id,
            &a.candidate_id,
        )
            .cmp(&(
                &b.peer_node_id,
                b.transport,
                &b.initiator_node_id,
                &b.listener_id,
                &b.candidate_id,
            ))
    });
    result
}

async fn accept(
    runtime: Arc<Runtime>,
    listener: TcpListener,
    id: String,
    peers: Vec<String>,
    profile: profiles::Server,
    handshakes: Arc<Semaphore>,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(profile.tls);
    let mut pending = JoinSet::new();
    loop {
        tokio::select! {
            incoming = listener.accept() => {
                let Ok((socket, remote)) = incoming else { return; };
                if pending.len() >= 32 || socket.set_nodelay(true).is_err() { continue; }
                let Ok(permit) = handshakes.clone().try_acquire_owned() else { continue; };
                let runtime = runtime.clone();
                let acceptor = acceptor.clone();
                let adapter = profile.vless.clone();
                let context = StreamContext { listener_id: Some(id.clone()), allowed_peers: Some(peers.clone()),
                    remote_addr: Some(remote), ..Default::default() };
                pending.spawn(async move {
                    let _permit = permit;
                    let _ = time::timeout(Duration::from_secs(5), async {
                        let mut stream = acceptor.accept(socket).await.map_err(|_| ())?;
                        meshway_transport_vless::server_handshake(&mut stream, &adapter).await.map_err(|_| ())?;
                        runtime.attach_stream_with_context(stream, StreamRole::Acceptor, TransportKind::Vless, 0, context)
                            .await.map_err(|_| ())
                    }).await;
                });
            }
            _ = pending.join_next(), if !pending.is_empty() => {},
        }
    }
}

async fn dial(
    runtime: Arc<Runtime>,
    candidate: OutgoingConnectionCandidate,
    addresses: Vec<SocketAddr>,
    profile: profiles::Client,
    priorities: Arc<BTreeMap<String, u16>>,
    outcomes: Outcomes,
    handshakes: Arc<Semaphore>,
) {
    let connector = tokio_rustls::TlsConnector::from(profile.tls);
    loop {
        let current = runtime
            .peer_connections()
            .into_iter()
            .find(|peer| peer.peer_node_id == candidate.peer_node_id);
        let should_dial = current.is_none_or(|peer| {
            peer.initiator
                && peer
                    .candidate_id
                    .as_ref()
                    .and_then(|id| priorities.get(id))
                    .is_some_and(|priority| *priority > candidate.priority)
        });
        if should_dial {
            let mut outcome = Reason::TransportFailed;
            for address in &addresses {
                let connected = time::timeout(Duration::from_secs(5), async {
                    let _permit = handshakes
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| Reason::InternalError)?;
                    let socket = TcpStream::connect(address)
                        .await
                        .map_err(|_| Reason::TransportFailed)?;
                    socket
                        .set_nodelay(true)
                        .map_err(|_| Reason::TransportFailed)?;
                    let remote_addr = socket.peer_addr().ok();
                    let stream = connector
                        .connect(profile.name.clone(), socket)
                        .await
                        .map_err(|error| {
                            if error
                                .get_ref()
                                .and_then(|error| error.downcast_ref::<rustls::Error>())
                                .is_some()
                            {
                                Reason::AuthenticationFailed
                            } else {
                                Reason::TransportFailed
                            }
                        })?;
                    let stream = meshway_transport_vless::client_stream(stream, &profile.vless)
                        .await
                        .map_err(|_| Reason::TransportFailed)?;
                    runtime
                        .attach_stream_with_context(
                            stream,
                            StreamRole::Dialer {
                                peer: candidate.peer_node_id.clone(),
                            },
                            TransportKind::Vless,
                            candidate.priority,
                            StreamContext {
                                candidate_id: Some(candidate.id.clone()),
                                remote_addr,
                                ..Default::default()
                            },
                        )
                        .await
                        .map_err(|error| match error {
                            meshway_dataplane::Error::Capacity
                            | meshway_dataplane::Error::Unavailable
                            | meshway_dataplane::Error::Timeout
                            | meshway_dataplane::Error::Closed => Reason::TransportFailed,
                            meshway_dataplane::Error::InvalidConfig(_) => Reason::InternalError,
                            _ => Reason::AuthenticationFailed,
                        })
                })
                .await;
                let reason = match &connected {
                    Ok(Ok(())) => Reason::None,
                    Ok(Err(reason)) => *reason,
                    Err(_) => Reason::TransportFailed,
                };
                // One DNS candidate can contain reachable and refused address
                // families. Preserve an actual authentication rejection from
                // this attempt set instead of hiding it behind a later refusal.
                if reason == Reason::None || outcome != Reason::AuthenticationFailed {
                    outcome = reason;
                }
                if matches!(connected, Ok(Ok(()))) {
                    break;
                }
            }
            outcomes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(candidate.id.clone(), outcome);
        }
        time::sleep(Duration::from_millis(500)).await;
    }
}
