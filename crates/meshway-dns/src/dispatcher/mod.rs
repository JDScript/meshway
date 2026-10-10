//! The exchange adapter is the host's approved local business socket and
//! SecureNode service path. It passes no credentials or bootstrap grants.
use crate::{
    Answer, Begin, Limits, LocalResolverRoute, MeshResolverRoute, Owner, Query, ResolverRoute,
    ResponseTransport, Ticket, TrustedUpstreamResponse, Unavailable, UpstreamWork,
};
use bytes::Bytes;
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::DnsTransport;
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio::sync::{Mutex, watch};

pub type DnsFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("DNS gateway operation failed")]
pub struct GatewayError;

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

/// The installed netmap and the system resolver observation a query owner is
/// created for. Each resolver endpoint is decided by the netmap itself
/// (`dns_route::netmap_resolver` and the source egress partition).
#[derive(Clone)]
pub struct Scope {
    pub snapshot: ExecutionSnapshot,
    pub system_resolvers: Option<crate::SystemResolverSnapshot>,
}

impl Scope {
    pub fn netmap(
        snapshot: ExecutionSnapshot,
        system_resolvers: Option<crate::SystemResolverSnapshot>,
    ) -> Self {
        Self {
            snapshot,
            system_resolvers,
        }
    }

    /// The same netmap content and the same system resolver observation:
    /// a lease renewal or new instance binding replaces nothing.
    pub fn equivalent(&self, other: &Self) -> bool {
        self.snapshot.same_netmap(&other.snapshot)
            && match (&self.system_resolvers, &other.system_resolvers) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same(b),
                _ => false,
            }
    }

    /// Installation readiness, distinct from per-query selection. Every
    /// declared resolver and captured system endpoint must be decided by the
    /// netmap.
    pub fn validate_resolvers(&self) -> Result<(), DispatchError> {
        use crate::owner::Plan;
        let plan = Plan::of(&self.snapshot).ok_or(DispatchError::Scope)?;
        if plan == Plan::Unavailable {
            return Err(DispatchError::Unavailable(Unavailable::MissingExitPlan));
        }
        let via_exit = matches!(plan, Plan::ViaExit { .. });
        if via_exit {
            for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
                MeshResolverRoute::new(
                    self.snapshot.clone(),
                    transport,
                    self.snapshot.expires_at(),
                )
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingExitPlan))?;
            }
            if self.system_resolvers.is_some() {
                return Err(DispatchError::Scope);
            }
        }
        let netmap = self.snapshot.netmap().map_err(|_| DispatchError::Scope)?;
        for (key, server) in &netmap.netmap.projection.configuration.dns.servers {
            if via_exit && !server.use_with_exit {
                continue;
            }
            LocalResolverRoute::new(self.snapshot.clone(), key, self.snapshot.expires_at())
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
        }
        if let Some(system) = &self.system_resolvers {
            let route = |index, transport| {
                LocalResolverRoute::system(
                    self.snapshot.clone(),
                    system.clone(),
                    index,
                    transport,
                    self.snapshot.expires_at(),
                )
                .map(|_| ())
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))
            };
            for index in 0..system.upstreams.len() {
                route(index, system.primary)?;
                if system.primary == DnsTransport::Udp {
                    route(index, DnsTransport::Tcp)?;
                }
            }
        }
        Ok(())
    }
}

/// An additional cancellation fence, never a grant or a replacement lease. It
/// ends when the published scope changes to one that is not equivalent (a new
/// netmap serial, a readiness withdrawal or a new system resolver
/// observation); a same-content renewal leaves it current.
#[derive(Clone)]
pub(crate) struct ScopeFence {
    scopes: watch::Receiver<Option<Scope>>,
    original: Scope,
}

impl ScopeFence {
    fn new(scopes: watch::Receiver<Option<Scope>>, original: Scope) -> Self {
        Self { scopes, original }
    }
    pub(crate) fn current(&self) -> bool {
        match self.scopes.has_changed() {
            Ok(false) => true,
            Ok(true) => self
                .scopes
                .borrow()
                .as_ref()
                .is_some_and(|scope| scope.equivalent(&self.original)),
            Err(_) => false,
        }
    }
    pub(crate) async fn invalidated(&self) {
        let mut receiver = self.scopes.clone();
        loop {
            if !self.current() || receiver.changed().await.is_err() {
                return;
            }
            if !receiver
                .borrow_and_update()
                .as_ref()
                .is_some_and(|scope| scope.equivalent(&self.original))
            {
                return;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    #[error("DNS scope is unavailable or changed")]
    Scope,
    #[error("DNS work expired")]
    Expired,
    #[error("DNS upstream failed")]
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
    /// A query a Mesh source sent to `local`. Only a resolver Node's served
    /// plan accepts one; loopback clients use `dispatch`.
    fn dispatch_from(
        &self,
        _peer: std::net::SocketAddr,
        _local: std::net::SocketAddr,
        _transport: DnsTransport,
        _query: Query,
        _deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>> {
        Box::pin(async { Err(DispatchError::Scope) })
    }
    fn stop(&self) -> DnsFuture<'_, ()>;
}

struct State {
    scopes: watch::Receiver<Option<Scope>>,
    owner: Option<Owner>,
    /// The scope the current owner was created for.
    installed: Option<Scope>,
    stopped: bool,
}

/// One query owner shared by UDP and every TCP connection. No background task,
/// credential, cache, independent lease or socket is owned by this dispatcher.
pub struct OwnerDispatcher {
    state: Mutex<State>,
    served: served::Served,
    exchange: Arc<dyn ResolverExchange>,
    limits: Limits,
    udp_payload_limit: u16,
}

impl OwnerDispatcher {
    /// Installation checks include the real adapter. A syntactically valid
    /// ViaExit plan cannot be reported applied with the local-only adapter.
    pub fn validate_scope(&self, scope: &Scope) -> Result<(), DispatchError> {
        use crate::owner::Plan;
        scope.validate_resolvers()?;
        if let Plan::ViaExit { .. } = Plan::of(&scope.snapshot).ok_or(DispatchError::Scope)? {
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
        Ok(())
    }
    pub fn new(
        scopes: watch::Receiver<Option<Scope>>,
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
            served: served::Served::default(),
            state: Mutex::new(State {
                scopes,
                owner: None,
                installed: None,
                stopped: false,
            }),
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
            state.installed = None;
            return Err(DispatchError::Scope);
        }
        // Read the changed flag under the same watch borrow as the value. A
        // send between has_changed() and borrow_and_update() must not mark a
        // replacement seen while retaining the previous query owner.
        let (changed, scope) = {
            let current = state.scopes.borrow_and_update();
            let scope = current.as_ref().cloned();
            (current.has_changed(), scope)
        };
        let Some(scope) = scope else {
            if let Some(owner) = state.owner.as_mut() {
                owner.stop();
            }
            state.owner = None;
            state.installed = None;
            return Err(DispatchError::Scope);
        };
        let equivalent = state
            .installed
            .as_ref()
            .is_some_and(|installed| installed.equivalent(&scope));
        if (changed && !equivalent) || state.owner.is_none() {
            // A new netmap serial cancels old tickets. No old operation is
            // completed under a transplanted generation. Same netmap content
            // keeps the owner: its snapshot already carries the renewed lease.
            state.owner = None;
            state.installed = None;
            state.owner = Some(
                Owner::new(scope.snapshot.clone(), self.limits)
                    .map_err(|_| DispatchError::Scope)?,
            );
            state.installed = Some(scope.clone());
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
        let (begin, fence) = {
            let mut state = self.state.lock().await;
            let scope = self.reconcile_state(&mut state)?;
            if Instant::now() >= deadline {
                return Err(DispatchError::Expired);
            }
            let mut begin = state
                .owner
                .as_mut()
                .ok_or(DispatchError::Scope)?
                .begin_published_until(query.clone(), deadline);
            if matches!(begin, Begin::Unmanaged)
                && let Some(system) = &scope.system_resolvers
            {
                begin = state
                    .owner
                    .as_mut()
                    .ok_or(DispatchError::Scope)?
                    .begin_system(query, system, deadline);
            }
            let fence = ScopeFence::new(state.scopes.clone(), scope);
            (begin, fence)
        };
        match begin {
            Begin::Answer(answer) => Ok(answer.with_scope_fence(fence)),
            Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
            Begin::Unmanaged => Err(DispatchError::PreserveSystemResolution),
            Begin::Upstream(work) => self.upstream(work, fence).await,
        }
    }

    /// Run one admitted upstream query to completion on its sealed routes:
    /// ordered failover only to independently authorized upstreams and the
    /// UDP→TCP retry, all within the original ticket.
    async fn upstream(
        &self,
        mut work: Box<UpstreamWork>,
        fence: ScopeFence,
    ) -> Result<Answer, DispatchError> {
        let mut cancellation = CancelOnDrop(Some(work.ticket.clone()));
        let mut attempt_deadline =
            upstream_deadline(work.ticket.expires_at(), work.remaining.len() + 1);
        let response = loop {
            // This is the original grant, not the shorter exchange
            // budget. A permission failure must never trigger failover.
            work.route
                .ensure_current()
                .map_err(|_| DispatchError::Expired)?;
            if !fence.current() {
                return Err(DispatchError::Scope);
            }
            let query = work
                .query
                .encode_upstream(self.udp_payload_limit)
                .map_err(|_| DispatchError::Wire)?;
            let result = {
                let exchange = ExchangeScope(work.route.exchange_scope(attempt_deadline));
                tokio::select! { biased;
                    _ = fence.invalidated() => return Err(DispatchError::Scope),
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
            if !fence.current() {
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
                    attempt_deadline =
                        upstream_deadline(work.ticket.expires_at(), work.remaining.len() + 1);
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
        Ok(answer.with_scope_fence(fence))
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

mod served;
#[cfg(test)]
mod tests;
