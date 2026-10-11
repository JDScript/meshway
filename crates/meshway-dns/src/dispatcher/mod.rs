//! The exchange adapter is the host's approved local business socket and
//! SecureNode service path. It passes no credentials or bootstrap grants.
use crate::{
    Answer, Begin, Limits, LocalResolverRoute, MeshResolverRoute, Owner, Query, ResolverRoute,
    ResponseTransport, Ticket, TrustedUpstreamResponse, Unavailable, UpstreamWork,
};
use bytes::Bytes;
use meshway_configuration::DnsTransport;
use meshway_engine::Desired;
use std::{future::Future, pin::Pin, sync::Arc, time::Instant};
use tokio::sync::{Mutex, watch};
use tokio_util::sync::CancellationToken;

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

/// The desired netmap and the system resolver observation a query owner is
/// created for. Each resolver endpoint is decided by the netmap itself
/// (`dns_route::netmap_resolver` and the source egress partition). Its
/// publisher ends it with [`publish`]: every ticket, route and answer of
/// its owner then ends at once.
#[derive(Clone)]
pub struct Scope {
    pub desired: Desired,
    /// This Node's own OS upstreams for its unmanaged names.
    pub system_resolvers: Option<crate::SystemResolverSnapshot>,
    /// An exit resolver's OS upstreams for its sources
    /// ([`crate::exit_resolvers`]); never used for this Node's own names.
    pub served_resolvers: Option<crate::SystemResolverSnapshot>,
    ended: CancellationToken,
}

impl Scope {
    pub fn netmap(
        desired: Desired,
        system_resolvers: Option<crate::SystemResolverSnapshot>,
    ) -> Self {
        Self {
            desired,
            system_resolvers,
            served_resolvers: None,
            ended: CancellationToken::new(),
        }
    }

    /// The same scope with an exit resolver's served upstreams.
    pub fn serving(mut self, served: Option<crate::SystemResolverSnapshot>) -> Self {
        self.served_resolvers = served;
        self
    }

    /// The same content and the same system resolver observations.
    pub fn equivalent(&self, other: &Self) -> bool {
        let same = |a: &Option<crate::SystemResolverSnapshot>,
                    b: &Option<crate::SystemResolverSnapshot>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        };
        self.desired.reference() == other.desired.reference()
            && same(&self.system_resolvers, &other.system_resolvers)
            && same(&self.served_resolvers, &other.served_resolvers)
    }

    /// Installation readiness, distinct from per-query selection. Every
    /// declared resolver and captured system endpoint must be decided by the
    /// netmap.
    pub fn validate_resolvers(&self) -> Result<(), DispatchError> {
        use crate::owner::Plan;
        let plan = Plan::of(&self.desired);
        if plan == Plan::Unavailable {
            return Err(DispatchError::Unavailable(Unavailable::MissingExitPlan));
        }
        if self.served_resolvers.is_some() && plan != Plan::ServeExitResolver {
            return Err(DispatchError::Scope);
        }
        let via_exit = matches!(plan, Plan::ViaExit { .. });
        if via_exit {
            for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
                MeshResolverRoute::new(&self.desired, transport, Instant::now() + VALIDATION)
                    .map_err(|_| DispatchError::Unavailable(Unavailable::MissingExitPlan))?;
            }
            if self.system_resolvers.is_some() {
                return Err(DispatchError::Scope);
            }
        }
        for (key, server) in &self.desired.netmap().projection.configuration.dns.servers {
            if via_exit && !server.use_with_exit {
                continue;
            }
            LocalResolverRoute::new(&self.desired, key, Instant::now() + VALIDATION)
                .map_err(|_| DispatchError::Unavailable(Unavailable::MissingResolverPlan))?;
        }
        if let Some(system) = &self.system_resolvers {
            let route = |index, transport| {
                LocalResolverRoute::system(
                    &self.desired,
                    system.clone(),
                    index,
                    transport,
                    Instant::now() + VALIDATION,
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

/// Construction-only budget of a validation route; nothing is sent on it.
const VALIDATION: std::time::Duration = std::time::Duration::from_secs(1);

/// Publish `next` (`None`: withdraw). An equivalent scope keeps the current
/// owner; anything else ends the previous scope's work before replacing it.
pub fn publish(scopes: &watch::Sender<Option<Scope>>, next: Option<Scope>) {
    scopes.send_if_modified(|current| {
        if let (Some(old), Some(new)) = (current.as_ref(), next.as_ref())
            && old.equivalent(new)
        {
            return false;
        }
        if let Some(old) = current.take() {
            old.ended.cancel();
        }
        *current = next;
        true
    });
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
        if let Plan::ViaExit { .. } = Plan::of(&scope.desired) {
            for transport in [DnsTransport::Udp, DnsTransport::Tcp] {
                let route =
                    MeshResolverRoute::new(&scope.desired, transport, Instant::now() + VALIDATION)
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
    /// automatically; `publish` ends old output before this is scheduled.
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
        // A withdrawn scope's owner never serves a republished one, even with
        // the same content: its work ended with that scope.
        let equivalent = state.installed.as_ref().is_some_and(|installed| {
            installed.equivalent(&scope) && !installed.ended.is_cancelled()
        });
        if (changed && !equivalent) || state.owner.is_none() {
            // New content cancels old tickets. No old operation is completed
            // under a transplanted generation.
            state.owner = None;
            state.installed = None;
            state.owner = Some(
                Owner::new(&scope.desired, self.limits, &scope.ended)
                    .map_err(|_| DispatchError::Scope)?,
            );
            state.installed = Some(scope.clone());
        }
        if scope.ended.is_cancelled() {
            return Err(DispatchError::Scope);
        }
        Ok(scope)
    }

    async fn execute(&self, query: Query, deadline: Instant) -> Result<Answer, DispatchError> {
        if Instant::now() >= deadline {
            return Err(DispatchError::Expired);
        }
        let begin = {
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
            begin
        };
        match begin {
            Begin::Answer(answer) => Ok(answer),
            Begin::Unavailable(reason) => Err(DispatchError::Unavailable(reason)),
            Begin::Unmanaged => Err(DispatchError::PreserveSystemResolution),
            Begin::Upstream(work) => self.upstream(work).await,
        }
    }

    /// Run one admitted upstream query to completion on its sealed routes:
    /// ordered failover only to independently authorized upstreams and the
    /// UDP→TCP retry, all within the original ticket.
    /// The ticket is a child of the scope: its withdrawal ends this work.
    async fn upstream(&self, mut work: Box<UpstreamWork>) -> Result<Answer, DispatchError> {
        let mut cancellation = CancelOnDrop(Some(work.ticket.clone()));
        let mut attempt_deadline =
            upstream_deadline(work.ticket.expires_at(), work.remaining.len() + 1);
        let response = loop {
            // This is the original grant, not the shorter exchange
            // budget. A permission failure must never trigger failover.
            work.route
                .ensure_current()
                .map_err(|_| DispatchError::Expired)?;
            let query = work
                .query
                .encode_upstream(self.udp_payload_limit)
                .map_err(|_| DispatchError::Wire)?;
            let result = {
                let exchange = ExchangeScope(work.route.exchange_scope(attempt_deadline));
                tokio::select! { biased;
                    _ = work.ticket.cancelled() => return Err(DispatchError::Expired),
                    _ = work.route.ended() => return Err(DispatchError::Expired),
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
        Ok(answer)
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
