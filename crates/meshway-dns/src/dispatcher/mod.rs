//! Gateways delegate to the host's unique Control driver and approved local
//! business socket adapter. Neither trait passes credentials or bootstrap grants.
use crate::{
    Answer, Begin, Limits, LocalResolverRoute, MeshResolverRoute, Owner, Query, ResolverRoute,
    ResponseTransport, Ticket, TrustedUpstreamResponse, Unavailable,
};
use bytes::Bytes;
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::DnsTransport;
use meshway_model::RequestId;
use meshway_protocol::NodeDnsQueryResponse;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio::sync::{Mutex, watch};

pub type DnsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("DNS gateway operation failed")]
pub struct GatewayError;

/// Implement using a bounded gateway to the existing unique Control driver.
/// Return only its authenticated typed `Control::query_dns` result. Cancellation
/// must cancel queued work; no cloned credentials or independent HTTP owner.
pub trait DiscoveryGateway: Send + Sync + 'static {
    fn query<'a>(
        &'a self,
        name: &'a str,
    ) -> DnsFuture<'a, Result<NodeDnsQueryResponse, GatewayError>>;
}

/// Exactly one DNS exchange on the sealed route. Payloads exclude TCP framing.
/// Validate actual source endpoint, transaction and frame/payload bounds before
/// accepting bytes. Enforce route validity at every socket I/O poll, including
/// connect/write/read. Prefer future-owned I/O; if a worker accepts the work,
/// retain its route cancellation guard and join it during adapter shutdown.
/// ID randomization is adapter-owned; restore the input ID after validation.
/// No UDP-to-TCP retry is authorized by a UDP route.
pub trait ResolverExchange: Send + Sync + 'static {
    /// Must prove an installed service-purpose path on the unique SecureNode.
    /// Default adapters explicitly lack Mesh support; never use local sockets.
    fn validate_mesh(&self, _route: &MeshResolverRoute) -> Result<(), GatewayError> {
        Err(GatewayError)
    }
    fn exchange_mesh<'a>(
        &'a self,
        _route: &'a MeshResolverRoute,
        _query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async { Err(GatewayError) })
    }
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>>;
}

#[derive(Clone)]
pub struct Scope {
    pub snapshot: ExecutionSnapshot,
    /// Selection of demand IDs only. Each endpoint/protocol is checked against
    /// the compiler-produced local_egress binding in this exact snapshot. At most
    /// 256 mappings are accepted (the compiler's current demand budget).
    pub resolver_demands: BTreeMap<String, RequestId>,
    pub system_resolvers: Option<crate::SystemResolverSnapshot>,
    pub system_demands: BTreeMap<usize, RequestId>,
    pub system_tcp_demands: BTreeMap<usize, RequestId>,
}

impl Scope {
    /// Installation readiness, distinct from per-query selection. Every declared
    /// resolver and captured system endpoint must have an exact current permit.
    pub fn validate_resolvers(&self) -> Result<(), DispatchError> {
        let cfg = self
            .snapshot
            .configuration()
            .map_err(|_| DispatchError::Scope)?;
        if matches!(
            cfg.dns.egress,
            meshway_configuration::execution::DnsEgress::Unavailable { .. }
        ) {
            return Err(DispatchError::Unavailable(Unavailable::MissingExitPlan));
        }
        if self.resolver_demands.len() + self.system_demands.len() + self.system_tcp_demands.len()
            > 256
        {
            return Err(DispatchError::Scope);
        }
        let via_exit = matches!(
            cfg.dns.egress,
            meshway_configuration::execution::DnsEgress::ViaExit { .. }
        );
        if via_exit {
            for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
                MeshResolverRoute::new(
                    self.snapshot.clone(),
                    transport,
                    self.snapshot.expires_at(),
                )
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingExitPlan))?;
            }
            if self.system_resolvers.is_some()
                || !self.system_demands.is_empty()
                || !self.system_tcp_demands.is_empty()
            {
                return Err(DispatchError::Scope);
            }
        }
        for (key, server) in &cfg.dns.settings.servers {
            if via_exit && !server.use_with_exit {
                continue;
            }
            let id = self
                .resolver_demands
                .get(key)
                .ok_or(DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
            LocalResolverRoute::new(self.snapshot.clone(), key, *id, self.snapshot.expires_at())
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
        }
        if let Some(system) = &self.system_resolvers {
            for index in 0..system.upstreams.len() {
                let id = self
                    .system_demands
                    .get(&index)
                    .ok_or(DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
                LocalResolverRoute::system(
                    self.snapshot.clone(),
                    system.clone(),
                    index,
                    system.primary,
                    *id,
                    self.snapshot.expires_at(),
                )
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
                if system.primary == DnsTransport::Udp {
                    let id = self
                        .system_tcp_demands
                        .get(&index)
                        .ok_or(DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
                    LocalResolverRoute::system(
                        self.snapshot.clone(),
                        system.clone(),
                        index,
                        DnsTransport::Tcp,
                        *id,
                        self.snapshot.expires_at(),
                    )
                    .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
                }
            }
        } else if !self.system_demands.is_empty() || !self.system_tcp_demands.is_empty() {
            return Err(DispatchError::Scope);
        }
        Ok(())
    }
}

/// An additional cancellation fence, never a grant or a replacement lease.
pub(crate) struct ScopeFence(watch::Receiver<Option<Scope>>);

impl ScopeFence {
    pub(crate) fn current(&self) -> bool {
        matches!(self.0.has_changed(), Ok(false))
    }
    pub(crate) async fn invalidated(&self) {
        let mut receiver = self.0.clone();
        let _ = receiver.changed().await;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    #[error("DNS scope is unavailable or changed")]
    Scope,
    #[error("DNS work expired")]
    Expired,
    #[error("DNS upstream or discovery failed")]
    Gateway,
    #[error("DNS result is malformed")]
    Wire,
    #[error("ordinary system resolution must be preserved; no approved adapter is installed")]
    PreserveSystemResolution,
    #[error("DNS plan unavailable: {0:?}")]
    Unavailable(Unavailable),
}

/// A server handler can return only an owner-guarded answer or a typed failure.
/// The service generates its own bounded empty SERVFAIL for failures.
pub trait Dispatcher: Send + Sync + 'static {
    fn dispatch(
        &self,
        query: Query,
        deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>>;
    fn stop(&self) -> DnsFuture<'_, ()>;
}

struct State {
    scopes: watch::Receiver<Option<Scope>>,
    owner: Option<Owner>,
    stopped: bool,
}

/// One query owner shared by UDP and every TCP connection. No background task,
/// credential, cache, independent lease or socket is owned by this dispatcher.
pub struct OwnerDispatcher {
    state: Mutex<State>,
    discovery: Arc<dyn DiscoveryGateway>,
    exchange: Arc<dyn ResolverExchange>,
    limits: Limits,
    udp_payload_limit: u16,
}

impl OwnerDispatcher {
    /// Installation checks include the real adapter. A syntactically valid
    /// ViaExit plan cannot be reported applied with the local-only adapter.
    pub fn validate_scope(&self, scope: &Scope) -> Result<(), DispatchError> {
        scope.validate_resolvers()?;
        let cfg = scope
            .snapshot
            .configuration()
            .map_err(|_| DispatchError::Scope)?;
        match cfg.dns.egress {
            meshway_configuration::execution::DnsEgress::ViaExit { .. } => {
                for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
                    let route = MeshResolverRoute::new(
                        scope.snapshot.clone(),
                        transport,
                        scope.snapshot.expires_at(),
                    )
                    .map_err(|_| DispatchError::Scope)?;
                    self.exchange
                        .validate_mesh(&route)
                        .map_err(|_| DispatchError::Gateway)?;
                }
            }
            // A local-loopback dispatcher cannot prove authenticated ingress.
            meshway_configuration::execution::DnsEgress::ServeExitResolver { .. } => {
                return Err(DispatchError::Unavailable(Unavailable::MissingExitPlan));
            }
            _ => {}
        }
        Ok(())
    }
    pub fn new(
        scopes: watch::Receiver<Option<Scope>>,
        discovery: Arc<dyn DiscoveryGateway>,
        exchange: Arc<dyn ResolverExchange>,
        limits: Limits,
        udp_payload_limit: u16,
    ) -> Result<Self, DispatchError> {
        if limits.max_pending == 0
            || limits.max_pending > 4096
            || limits.query_timeout.is_zero()
            || Instant::now().checked_add(limits.query_timeout).is_none()
            || udp_payload_limit < 512
        {
            return Err(DispatchError::Scope);
        }
        Ok(Self {
            state: Mutex::new(State {
                scopes,
                owner: None,
                stopped: false,
            }),
            discovery,
            exchange,
            limits,
            udp_payload_limit,
        })
    }

    /// Validate/install the latest query-owner scope. This is not proof that all
    /// resolver routes or host DNS resources are installed. Queries also reconcile
    /// automatically; notifications fence old output before this is scheduled.
    pub async fn reconcile(&self) -> Result<(), DispatchError> {
        let mut state = self.state.lock().await;
        self.reconcile_state(&mut state).map(|_| ())
    }

    fn reconcile_state(&self, state: &mut State) -> Result<Scope, DispatchError> {
        if state.stopped {
            return Err(DispatchError::Scope);
        }
        if state.scopes.has_changed().is_err() {
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
            return Err(DispatchError::Scope);
        }
        // Read the changed flag under the same watch borrow as the value. A
        // send between has_changed() and borrow_and_update() must not mark a
        // replacement seen while retaining the previous query owner.
        let (changed, scope) = {
            let current = state.scopes.borrow_and_update();
            let scope = current
                .as_ref()
                .filter(|scope| {
                    scope.resolver_demands.len()
                        + scope.system_demands.len()
                        + scope.system_tcp_demands.len()
                        <= 256
                })
                .cloned();
            (current.has_changed(), scope)
        };
        let Some(scope) = scope else {
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
            return Err(DispatchError::Scope);
        };
        if changed || state.owner.is_none() {
            // Even same-target renewal cancels old tickets. No old operation is
            // completed under a newly extended scope or transplanted generation.
            state.owner = None;
            state.owner = Some(
                Owner::new(scope.snapshot.clone(), self.limits)
                    .map_err(|_| DispatchError::Scope)?,
            );
        }
        scope
            .snapshot
            .ensure_current()
            .map_err(|_| DispatchError::Scope)?;
        Ok(scope)
    }

    async fn execute(&self, query: Query, deadline: Instant) -> Result<Answer, DispatchError> {
        if Instant::now() >= deadline {
            return Err(DispatchError::Expired);
        }
        let (begin, mut scopes, snapshot) = {
            let mut state = self.state.lock().await;
            let scope = self.reconcile_state(&mut state)?;
            if Instant::now() >= deadline {
                return Err(DispatchError::Expired);
            }
            let mut begin = state
                .owner
                .as_mut()
                .ok_or(DispatchError::Scope)?
                .begin_published_until(query.clone(), &scope.resolver_demands, deadline);
            if matches!(begin, Begin::Unmanaged)
                && let Some(system) = &scope.system_resolvers
            {
                begin = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .begin_system(
                        query,
                        system,
                        &scope.system_demands,
                        &scope.system_tcp_demands,
                        deadline,
                    );
            }
            (begin, state.scopes.clone(), scope.snapshot)
        };
        match begin {
            Begin::Answer(answer) => Ok(answer.with_scope_fence(ScopeFence(scopes))),
            Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
            Begin::Unmanaged => Err(DispatchError::PreserveSystemResolution),
            Begin::DiscoveryRequest(request) => {
                let mut cancellation = CancelOnDrop(Some(request.ticket.clone()));
                let response = tokio::select! { biased;
                    _ = scopes.changed() => return Err(DispatchError::Scope),
                    _ = snapshot.invalidated() => return Err(DispatchError::Scope),
                    _ = request.ticket.cancelled() => return Err(DispatchError::Expired),
                    _ = tokio::time::sleep_until(deadline.into()) => return Err(DispatchError::Expired),
                    result = self.discovery.query(&request.name) => result.map_err(|_| DispatchError::Gateway)?,
                };
                let mut state = self.state.lock().await;
                self.reconcile_state(&mut state)?;
                let result = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .complete_discovery(&request.ticket, response);
                let answer = completed(result)?;
                cancellation.0 = None;
                Ok(answer.with_scope_fence(ScopeFence(scopes)))
            }
            Begin::Upstream(mut work) => {
                let mut cancellation = CancelOnDrop(Some(work.ticket.clone()));
                let mut attempt_deadline =
                    upstream_deadline(work.ticket.expires_at(), work.remaining.len() + 1);
                let response = loop {
                    // This is the original grant, not the shorter exchange
                    // budget. A permission failure must never trigger failover.
                    work.route
                        .ensure_current()
                        .map_err(|_| DispatchError::Expired)?;
                    if !matches!(scopes.has_changed(), Ok(false)) {
                        return Err(DispatchError::Scope);
                    }
                    let query = work
                        .query
                        .encode_upstream(self.udp_payload_limit)
                        .map_err(|_| DispatchError::Wire)?;
                    let result = {
                        let exchange = ExchangeScope(work.route.exchange_scope(attempt_deadline));
                        tokio::select! { biased;
                            _ = scopes.changed() => return Err(DispatchError::Scope),
                            _ = work.ticket.cancelled() => return Err(DispatchError::Expired),
                            _ = work.route.invalidated() => return Err(DispatchError::Expired),
                            _ = tokio::time::sleep_until(attempt_deadline.into()) => Err(GatewayError),
                            result = async { match &exchange.0 {
                                ResolverRoute::Local(route) => self.exchange.exchange(route, query).await,
                                ResolverRoute::Mesh(route) => self.exchange.exchange_mesh(route, query).await,
                            } } => result,
                        }
                    };
                    // Completion can race withdrawal or a permit deadline. Do
                    // not reinterpret such a failure as a network timeout.
                    work.route
                        .ensure_current()
                        .map_err(|_| DispatchError::Expired)?;
                    if !matches!(scopes.has_changed(), Ok(false)) {
                        return Err(DispatchError::Scope);
                    }
                    let bytes = match result {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            let next = work.remaining.pop_front().ok_or(DispatchError::Gateway)?;
                            next.route
                                .ensure_current()
                                .map_err(|_| DispatchError::Expired)?;
                            if let Some(tcp) = &next.fallback {
                                tcp.ensure_current().map_err(|_| DispatchError::Expired)?;
                            }
                            work.route = ResolverRoute::Local(next.route);
                            work.fallback = next.fallback.map(ResolverRoute::Local);
                            attempt_deadline = upstream_deadline(
                                work.ticket.expires_at(),
                                work.remaining.len() + 1,
                            );
                            continue;
                        }
                    };
                    let transport = match work.route.transport() {
                        DnsTransport::Udp => ResponseTransport::Udp,
                        DnsTransport::Tcp => ResponseTransport::Tcp,
                    };
                    let response = TrustedUpstreamResponse::decode(
                        &work.query,
                        &bytes,
                        transport,
                        self.udp_payload_limit,
                    )
                    .map_err(|_| DispatchError::Wire)?;
                    if response.message.metadata.truncation
                        && let Some(fallback) = work.fallback.take()
                    {
                        work.route
                            .ensure_current()
                            .map_err(|_| DispatchError::Expired)?;
                        fallback
                            .ensure_current()
                            .map_err(|_| DispatchError::Expired)?;
                        work.route = fallback;
                        // UDP and its same-server TCP retry share one budget.
                        continue;
                    }
                    break response;
                };
                let mut state = self.state.lock().await;
                self.reconcile_state(&mut state)?;
                let result = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .complete_upstream(&work, response);
                let answer = completed(result)?;
                cancellation.0 = None;
                Ok(answer.with_scope_fence(ScopeFence(scopes)))
            }
        }
    }
}

impl Dispatcher for OwnerDispatcher {
    fn dispatch(
        &self,
        query: Query,
        deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>> {
        Box::pin(self.execute(query, deadline))
    }

    fn stop(&self) -> DnsFuture<'_, ()> {
        Box::pin(async {
            let mut state = self.state.lock().await;
            state.stopped = true;
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
        })
    }
}

fn completed(begin: Begin) -> Result<Answer, DispatchError> {
    match begin {
        Begin::Answer(answer) => Ok(answer),
        Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
        _ => Err(DispatchError::Scope),
    }
}

struct CancelOnDrop(Option<Ticket>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(ticket) = &self.0 {
            ticket.cancel();
        }
    }
}

fn upstream_deadline(deadline: Instant, remaining: usize) -> Instant {
    let now = Instant::now();
    // At most eight system upstreams. Divide the remaining original budget so
    // an unresponsive first resolver cannot consume every later resolver's time.
    now + deadline.saturating_duration_since(now) / remaining as u32
}

struct ExchangeScope(ResolverRoute);
impl Drop for ExchangeScope {
    fn drop(&mut self) {
        self.0.cancel_exchange();
    }
}

#[cfg(test)]
mod tests;
