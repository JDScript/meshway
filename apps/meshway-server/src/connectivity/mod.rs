//! Volatile bilateral acquisition, serialized with final SQL authorization.
//! Candidate facts live exclusively in StateHub. Pending changes are applied
//! only after the read transaction commits, on the same SQLite owner job.
pub(crate) mod maintenance;
use crate::{
    error::Error,
    execution::{
        Execution, NodePrincipal,
        connectivity::{Failure, ScopeRead, netmap_udp_coordination},
    },
    state::service::Runtime,
    store::Handle,
};
use chrono::Utc;
use meshway_model::TransportProtocol;
use meshway_model::{Digest, NodeId, UdpTraversalId, Version};
use meshway_protocol::connectivity::*;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

const PER_NODE: usize = 8;
const PER_NETWORK: usize = 256;
const PROCESS: usize = 4096;
type Result<T> = std::result::Result<T, Failure>;
type Reason = UdpTraversalUnavailableReason;

#[derive(Clone)]
enum Terminal {
    Cancelled,
    Unavailable(Reason),
}

#[derive(Clone)]
struct Attempt {
    request: PrepareUdpTraversalRequest,
    offer: UdpTraversalOffer,
    prepared_by: NodePrincipal,
    accepted: [Option<NodePrincipal>; 2],
    terminal: Option<Terminal>,
    ipv6: bool,
}
impl Attempt {
    fn participant(&self, node: NodeId) -> Option<usize> {
        if self.offer.local.candidate.socket.node_id == node {
            Some(0)
        } else if self.offer.peer.candidate.socket.node_id == node {
            Some(1)
        } else {
            None
        }
    }
    fn live(&self, now: u64) -> bool {
        self.terminal.is_none() && self.offer.expires_at_ms > now
    }
    fn phase(&self) -> UdpTraversalPhase {
        if self.accepted.iter().all(Option::is_some) {
            UdpTraversalPhase::Active
        } else {
            UdpTraversalPhase::Waiting
        }
    }
}

#[derive(Default)]
pub(crate) struct Attempts {
    entries: BTreeMap<UdpTraversalId, Attempt>,
}
impl Attempts {
    pub(crate) fn gc(&mut self, now: u64) {
        self.entries
            .retain(|_, entry| entry.request.expires_at_ms > now);
    }
    fn apply(&mut self, changes: Vec<Change>, now: u64) {
        self.gc(now);
        for change in changes {
            match change {
                Change::Insert(entry) => {
                    self.entries.insert(entry.offer.attempt_id, *entry);
                }
                Change::Accept(id, participant, principal) => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.accepted[participant] = Some(principal);
                    }
                }
                Change::End(id, terminal) => {
                    if let Some(entry) = self.entries.get_mut(&id) {
                        entry.terminal = Some(terminal);
                    }
                }
            }
        }
    }
}
enum Change {
    Insert(Box<Attempt>),
    Accept(UdpTraversalId, usize, NodePrincipal),
    End(UdpTraversalId, Terminal),
}
struct Plan<T> {
    value: T,
    changes: Vec<Change>,
}

/// SQL ownership prevents concurrent authority changes; this final clock check
/// also prevents a slow read/hash from returning a grant past its fixed deadline.
trait Reply {
    fn finish(&mut self, now: u64) -> std::result::Result<bool, Error>;
}
impl Reply for QueryUdpTraversalResponse {
    fn finish(&mut self, now: u64) -> std::result::Result<bool, Error> {
        if let Self::Current {
            expires_at_ms,
            attempts,
            ..
        } = self
        {
            if *expires_at_ms <= now {
                *self = Self::Unavailable {
                    reason: Reason::ScopeChanged,
                };
                return Ok(false);
            }
            *attempts = attempts
                .as_slice()
                .iter()
                .filter(|attempt| attempt.offer.expires_at_ms > now)
                .cloned()
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| Error::Internal("traversal query bound"))?;
        }
        Ok(true)
    }
}
impl Reply for PrepareUdpTraversalResponse {
    fn finish(&mut self, now: u64) -> std::result::Result<bool, Error> {
        if matches!(self, Self::Offer { offer } if offer.expires_at_ms <= now) {
            *self = Self::Unavailable {
                reason: Reason::AttemptExpired,
            };
            return Ok(false);
        }
        Ok(true)
    }
}
impl Reply for AcceptUdpTraversalResponse {
    fn finish(&mut self, now: u64) -> std::result::Result<bool, Error> {
        if matches!(self, Self::Waiting { offer } | Self::Active { offer } if offer.expires_at_ms <= now)
        {
            *self = Self::Unavailable {
                reason: Reason::AttemptExpired,
            };
            return Ok(false);
        }
        Ok(true)
    }
}
impl Reply for CancelUdpTraversalResponse {
    fn finish(&mut self, _: u64) -> std::result::Result<bool, Error> {
        Ok(true)
    }
}
impl<T> Plan<T> {
    fn read(value: T) -> Self {
        Self {
            value,
            changes: Vec::new(),
        }
    }
    fn change(value: T, change: Change) -> Self {
        Self {
            value,
            changes: vec![change],
        }
    }
}

#[derive(Clone)]
pub struct Connectivity {
    handle: Handle,
    execution: Execution,
    runtime: Arc<Mutex<Runtime>>,
    stun_servers: Arc<BTreeMap<String, meshway_transport_spec::Endpoint>>,
    stun_health: crate::stun::Health,
}
impl Connectivity {
    pub(crate) fn new(
        handle: Handle,
        execution: Execution,
        runtime: Arc<Mutex<Runtime>>,
        stun_servers: Arc<BTreeMap<String, meshway_transport_spec::Endpoint>>,
        stun_health: crate::stun::Health,
    ) -> Self {
        Self {
            handle,
            execution,
            runtime,
            stun_servers,
            stun_health,
        }
    }

    async fn run<T, F>(&self, token: &str, operation: F) -> std::result::Result<T, Error>
    where
        T: Reply + Send + 'static,
        F: FnOnce(
                &mut ScopeRead<'_, '_>,
                &NodePrincipal,
                &Runtime,
            ) -> std::result::Result<Plan<T>, Error>
            + Send
            + 'static,
    {
        let token = token.to_owned();
        let execution = self.execution.clone();
        let runtime = self.runtime.clone();
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let now = Utc::now();
                let principal = execution.traversal_principal(&tx, &token, now)?;
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                if runtime.is_closed() {
                    return Err(Error::CLOSED);
                }
                let mut scopes = ScopeRead::new(&execution, &tx, now);
                let mut plan = operation(&mut scopes, &principal, &runtime)?;
                tx.commit()?;
                let final_now = u64::try_from(Utc::now().timestamp_millis())
                    .map_err(|_| Error::Internal("system clock before epoch"))?;
                if !plan.value.finish(final_now)? {
                    plan.changes
                        .retain(|change| matches!(change, Change::End(..)));
                }
                // No await or other executor job
                // can intervene between the final checks and this memory mutation.
                runtime.traversal.apply(plan.changes, final_now);
                Ok(plan.value)
            })
            .await?
    }

    pub async fn query(
        &self,
        token: &str,
        request: QueryUdpTraversalRequest,
    ) -> std::result::Result<QueryUdpTraversalResponse, Error> {
        let servers = self.stun_servers.clone();
        let health = self.stun_health.clone();
        self.run(token, move |scopes, principal, runtime| {
            let now = scopes.now();
            let scope = match scopes.caller(principal, &request.instance, &request.netmap) {
                Ok(scope) => scope,
                Err(Failure::Unavailable(reason)) => {
                    return Ok(Plan::read(QueryUdpTraversalResponse::Unavailable {
                        reason,
                    }));
                }
                Err(Failure::Error(error)) => return Err(error),
            };
            if !health.ready() {
                return Ok(Plan::read(QueryUdpTraversalResponse::Unavailable {
                    reason: Reason::NotAvailable,
                }));
            }
            let mut attempts = Vec::new();
            let mut changes = Vec::new();
            for attempt in runtime.traversal.entries.values().filter(|a| {
                a.offer.network_id == principal.network_id()
                    && a.participant(principal.node_id()).is_some()
                    && a.terminal.is_none()
            }) {
                match recheck(scopes, runtime, attempt) {
                    Ok(()) if principal.expires_at_ms() >= attempt.offer.expires_at_ms => attempts
                        .push(UdpTraversalAttempt {
                            offer: attempt.offer.clone(),
                            phase: attempt.phase(),
                        }),
                    Ok(()) => {}
                    Err(Failure::Unavailable(reason)) => changes.push(Change::End(
                        attempt.offer.attempt_id,
                        Terminal::Unavailable(reason),
                    )),
                    Err(Failure::Error(error)) => return Err(error),
                }
            }
            let stun_servers = servers
                .iter()
                .map(|(id, endpoint)| StunServer {
                    id: id.clone(),
                    endpoint: endpoint.clone(),
                })
                .collect::<Vec<_>>()
                .try_into()
                .map_err(|_| Error::Internal("STUN metadata bound"))?;
            let attempts = attempts
                .try_into()
                .map_err(|_| Error::Internal("traversal query bound"))?;
            Ok(Plan {
                value: QueryUdpTraversalResponse::Current {
                    state_epoch: runtime.hub.epoch,
                    as_of_ms: now.timestamp_millis() as u64,
                    expires_at_ms: scope
                        .deadline_ms
                        .min(principal.expires_at_ms())
                        .min(now.timestamp_millis() as u64 + 30_000),
                    stun_servers,
                    attempts,
                },
                changes,
            })
        })
        .await
    }

    pub async fn prepare(
        &self,
        token: &str,
        request: PrepareUdpTraversalRequest,
    ) -> std::result::Result<PrepareUdpTraversalResponse, Error> {
        self.run(token, move |scopes, principal, runtime| {
            let previous = runtime
                .traversal
                .entries
                .values()
                .find(|entry| {
                    entry.terminal.is_none()
                        && entry.offer.network_id == principal.network_id()
                        && entry.prepared_by.node_id() == principal.node_id()
                        && entry.request == request
                })
                .map(|entry| entry.offer.attempt_id);
            match prepare(scopes, principal, runtime, request) {
                Ok((offer, change)) => Ok(Plan {
                    value: PrepareUdpTraversalResponse::Offer { offer },
                    changes: change.into_iter().collect(),
                }),
                Err(Failure::Unavailable(reason)) => Ok(Plan {
                    value: PrepareUdpTraversalResponse::Unavailable { reason },
                    changes: previous
                        .map(|id| Change::End(id, Terminal::Unavailable(reason)))
                        .into_iter()
                        .collect(),
                }),
                Err(Failure::Error(error)) => Err(error),
            }
        })
        .await
    }

    pub async fn accept(
        &self,
        token: &str,
        id: UdpTraversalId,
        request: UdpTraversalAttemptRequest,
    ) -> std::result::Result<AcceptUdpTraversalResponse, Error> {
        self.run(token, move |scopes, principal, runtime| {
            let mut check = || -> Result<(&Attempt, usize)> {
                let attempt = cas(scopes, principal, runtime, id, &request)?;
                let index = attempt
                    .participant(principal.node_id())
                    .ok_or(Reason::NotAvailable)?;
                Ok((attempt, index))
            };
            match check() {
                Ok((attempt, index)) => {
                    match recheck(scopes, runtime, attempt) {
                        Ok(()) => {}
                        Err(Failure::Unavailable(reason)) => {
                            let response = AcceptUdpTraversalResponse::Unavailable { reason };
                            return Ok(if attempt.terminal.is_none() {
                                Plan::change(
                                    response,
                                    Change::End(id, Terminal::Unavailable(reason)),
                                )
                            } else {
                                Plan::read(response)
                            });
                        }
                        Err(Failure::Error(error)) => return Err(error),
                    }
                    let active = attempt.accepted[1 - index].is_some();
                    let response = if active {
                        AcceptUdpTraversalResponse::Active {
                            offer: attempt.offer.clone(),
                        }
                    } else {
                        AcceptUdpTraversalResponse::Waiting {
                            offer: attempt.offer.clone(),
                        }
                    };
                    Ok(Plan::change(
                        response,
                        Change::Accept(id, index, principal.clone()),
                    ))
                }
                Err(Failure::Unavailable(reason)) => {
                    Ok(Plan::read(AcceptUdpTraversalResponse::Unavailable {
                        reason,
                    }))
                }
                Err(Failure::Error(error)) => Err(error),
            }
        })
        .await
    }

    pub async fn cancel(
        &self,
        token: &str,
        id: UdpTraversalId,
        request: UdpTraversalAttemptRequest,
    ) -> std::result::Result<CancelUdpTraversalResponse, Error> {
        self.run(token, move |scopes, principal, runtime| {
            match cas(scopes, principal, runtime, id, &request) {
                Ok(attempt) => match &attempt.terminal {
                    Some(Terminal::Unavailable(reason)) => {
                        Ok(Plan::read(CancelUdpTraversalResponse::Unavailable {
                            reason: *reason,
                        }))
                    }
                    _ => Ok(Plan::change(
                        CancelUdpTraversalResponse::Cancelled,
                        Change::End(id, Terminal::Cancelled),
                    )),
                },
                Err(Failure::Unavailable(reason)) => {
                    Ok(Plan::read(CancelUdpTraversalResponse::Unavailable {
                        reason,
                    }))
                }
                Err(Failure::Error(error)) => Err(error),
            }
        })
        .await
    }
}

fn cas<'a>(
    scopes: &mut ScopeRead<'_, '_>,
    principal: &NodePrincipal,
    runtime: &'a Runtime,
    id: UdpTraversalId,
    request: &UdpTraversalAttemptRequest,
) -> Result<&'a Attempt> {
    let now = scopes.now();
    scopes.caller(principal, &request.instance, &request.netmap)?;
    if request.state_epoch != runtime.hub.epoch {
        return Err(Reason::ScopeChanged.into());
    }
    let attempt = runtime
        .traversal
        .entries
        .get(&id)
        .ok_or(Reason::NotAvailable)?;
    if attempt.offer.network_id != principal.network_id()
        || attempt.participant(principal.node_id()).is_none()
    {
        return Err(Reason::NotAvailable.into());
    }
    let socket = if attempt.participant(principal.node_id()) == Some(0) {
        &attempt.offer.local.candidate.socket
    } else {
        &attempt.offer.peer.candidate.socket
    };
    if socket.instance != request.instance || socket.netmap != request.netmap {
        return Err(Reason::ScopeChanged.into());
    }
    if request.version != attempt.offer.version || request.digest != attempt.offer.digest {
        return Err(Reason::AttemptChanged.into());
    }
    if attempt.offer.expires_at_ms <= now.timestamp_millis() as u64 {
        return Err(Reason::AttemptExpired.into());
    }
    if principal.expires_at_ms() < attempt.offer.expires_at_ms {
        return Err(Reason::ScopeChanged.into());
    }
    Ok(attempt)
}

struct Pair {
    local: crate::state::connectivity::Candidate,
    peer: crate::state::connectivity::Candidate,
    deadline: u64,
}
fn pair(
    scopes: &mut ScopeRead<'_, '_>,
    runtime: &Runtime,
    principal: &NodePrincipal,
    request: &PrepareUdpTraversalRequest,
) -> Result<Pair> {
    let now = scopes.now();
    let local = scopes.caller(principal, &request.instance, &request.netmap)?;
    if request.state_epoch != runtime.hub.epoch {
        return Err(Reason::ScopeChanged.into());
    }
    if request.carrier.protocol != TransportProtocol::NativeUdp
        || request.peer_id == principal.node_id()
        || request.local_candidate.socket.node_id != principal.node_id()
        || request.peer_candidate.socket.node_id != request.peer_id
    {
        return Err(Reason::NotAvailable.into());
    }
    let peer = scopes.scope(principal.network_id(), request.peer_id)?;
    // Both current netmaps admit this Native UDP adjacency; no business flow
    // or complete path is involved. Candidates and acceptance stay bilateral.
    if !netmap_udp_coordination(&local.target.netmap, &peer.target.netmap, &request.carrier)? {
        return Err(Reason::NotAvailable.into());
    }
    let a =
        runtime
            .hub
            .traversal_candidate(&local, &request.carrier, &request.local_candidate, now)?;
    let b =
        runtime
            .hub
            .traversal_candidate(&peer, &request.carrier, &request.peer_candidate, now)?;
    if a.target.is_ipv6() != b.target.is_ipv6() {
        return Err(Reason::NotAvailable.into());
    }
    // Both participants' pools are checked even if configuration assembly ever
    // supplies distinct capture scopes. Underlay-local vetoes also run on Node.
    crate::state::connectivity::eligible_address(
        &a.address,
        &peer.target.netmap.self_.mesh_address_pools,
    )?;
    crate::state::connectivity::eligible_address(
        &b.address,
        &local.target.netmap.self_.mesh_address_pools,
    )?;
    let deadline = [
        request.expires_at_ms,
        local.deadline_ms,
        principal.expires_at_ms(),
        peer.deadline_ms,
        a.deadline_ms,
        b.deadline_ms,
    ]
    .into_iter()
    .min()
    .ok_or(Error::Internal("traversal deadline"))?;
    Ok(Pair {
        local: a,
        peer: b,
        deadline,
    })
}

fn recheck(scopes: &mut ScopeRead<'_, '_>, runtime: &Runtime, attempt: &Attempt) -> Result<()> {
    let now = scopes.now();
    if let Some(terminal) = &attempt.terminal {
        return Err(match terminal {
            Terminal::Cancelled => Reason::NotAvailable,
            Terminal::Unavailable(reason) => *reason,
        }
        .into());
    }
    if attempt.offer.expires_at_ms <= now.timestamp_millis() as u64 {
        return Err(Reason::AttemptExpired.into());
    }
    if scopes.reauthorize(&attempt.prepared_by)? < attempt.offer.expires_at_ms {
        return Err(Reason::ScopeChanged.into());
    }
    for principal in attempt.accepted.iter().flatten() {
        if scopes.reauthorize(principal)? < attempt.offer.expires_at_ms {
            return Err(Reason::ScopeChanged.into());
        }
    }
    let Pair {
        local: a,
        peer: b,
        deadline,
        ..
    } = pair(scopes, runtime, &attempt.prepared_by, &attempt.request)?;
    if deadline < attempt.offer.expires_at_ms
        || a.address != attempt.offer.local.address
        || b.address != attempt.offer.peer.address
        || a.source != attempt.offer.local.source
        || b.source != attempt.offer.peer.source
    {
        return Err(Reason::ScopeChanged.into());
    }
    Ok(())
}

fn prepare(
    scopes: &mut ScopeRead<'_, '_>,
    principal: &NodePrincipal,
    runtime: &Runtime,
    request: PrepareUdpTraversalRequest,
) -> Result<(UdpTraversalOffer, Option<Change>)> {
    let now = scopes.now();
    scopes.caller(principal, &request.instance, &request.netmap)?;
    if request.state_epoch != runtime.hub.epoch {
        return Err(Reason::ScopeChanged.into());
    }
    let now_ms = now.timestamp_millis() as u64;
    if let Some(attempt) = runtime.traversal.entries.values().find(|a| {
        a.offer.network_id == principal.network_id()
            && a.prepared_by.node_id() == principal.node_id()
            && a.request.request_id == request.request_id
            && a.request.expires_at_ms > now_ms
    }) {
        if attempt.request != request {
            return Err(Reason::RequestChanged.into());
        }
        recheck(scopes, runtime, attempt)?;
        return Ok((attempt.offer.clone(), None));
    }
    if request.expires_at_ms <= now_ms {
        return Err(Reason::AttemptExpired.into());
    }
    if request.expires_at_ms - now_ms > MAX_UDP_TRAVERSAL_LIFETIME_MS {
        return Err(Reason::NotAvailable.into());
    }
    let Pair {
        local: a,
        peer: b,
        deadline,
    } = pair(scopes, runtime, principal, &request)?;
    if deadline <= now_ms {
        return Err(Reason::AttemptExpired.into());
    }
    let records: Vec<_> = runtime
        .traversal
        .entries
        .values()
        .filter(|a| a.request.expires_at_ms > now_ms)
        .collect();
    if records.len() >= PROCESS
        || records
            .iter()
            .filter(|a| a.offer.network_id == principal.network_id())
            .count()
            >= PER_NETWORK
        || [principal.node_id(), request.peer_id]
            .into_iter()
            .any(|node| {
                records
                    .iter()
                    .filter(|a| {
                        a.offer.network_id == principal.network_id()
                            && a.participant(node).is_some()
                            && a.live(now_ms)
                    })
                    .count()
                    >= PER_NODE
            })
        || records.iter().any(|entry| {
            entry.live(now_ms)
                && entry.offer.network_id == principal.network_id()
                && entry.participant(principal.node_id()).is_some()
                && entry.participant(request.peer_id).is_some()
                && entry.ipv6 == a.target.is_ipv6()
        })
    {
        return Err(Reason::Capacity.into());
    }
    let attempt_id = UdpTraversalId::new(crate::accounts::random_uuid()?)
        .map_err(|_| Error::Internal("traversal ID"))?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::Internal("traversal randomness"))?;
    let mut offer = UdpTraversalOffer {
        attempt_id,
        request_id: request.request_id,
        network_id: principal.network_id(),
        state_epoch: runtime.hub.epoch,
        version: Version::MIN,
        digest: Digest::from_bytes([0; 32]),
        nonce: Digest::from_bytes(nonce),
        carrier: request.carrier.clone(),
        local: UdpTraversalParticipant {
            candidate: request.local_candidate.clone(),
            address: a.address,
            source: a.source,
        },
        peer: UdpTraversalParticipant {
            candidate: request.peer_candidate.clone(),
            address: b.address,
            source: b.source,
        },
        expires_at_ms: deadline,
    };
    offer.digest = offer
        .compute_digest()
        .map_err(|_| Error::encode("traversal offer"))?;
    let entry = Attempt {
        request,
        offer: offer.clone(),
        prepared_by: principal.clone(),
        accepted: [None, None],
        terminal: None,
        ipv6: a.target.is_ipv6(),
    };
    Ok((offer, Some(Change::Insert(Box::new(entry)))))
}
