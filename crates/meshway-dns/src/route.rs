use meshway_authority::ExecutionSnapshot;
use meshway_configuration::{
    DnsTransport,
    execution::LocalEgressAction,
    netmap::{EgressAction, NodeNetmap, ResolverAction},
};
use std::{fmt, net::SocketAddr, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("local resolver route is missing or does not match the netmap")]
    Binding,
    #[error("local resolver route is expired or cancelled")]
    Expired,
}

/// The one query lifetime both resolver route kinds share: the original
/// snapshot, a fixed permit cutoff, the request deadline and the query
/// ticket's cancellation. An exchange can only shorten it. It carries no
/// grant; each route kind keeps its own sealed one.
#[derive(Clone)]
pub(crate) struct QueryLifetime {
    snapshot: ExecutionSnapshot,
    expires_at_ms: u64,
    deadline: Instant,
    cancelled: CancellationToken,
}

impl QueryLifetime {
    pub(crate) fn new(snapshot: ExecutionSnapshot, expires_at_ms: u64, deadline: Instant) -> Self {
        Self {
            expires_at_ms: expires_at_ms.min(snapshot.expires_at_ms()),
            deadline: deadline.min(snapshot.expires_at()),
            snapshot,
            cancelled: CancellationToken::new(),
        }
    }
    pub(crate) fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    pub(crate) fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn bind_cancellation(&mut self, cancelled: CancellationToken) {
        self.cancelled = cancelled;
    }
    /// A child scope: the dispatcher can stop an accepted adapter worker
    /// without cancelling the query ticket that later, independently
    /// authorized upstreams still need.
    pub(crate) fn exchange(&self, deadline: Instant) -> Self {
        Self {
            deadline: self.deadline.min(deadline),
            cancelled: self.cancelled.child_token(),
            ..self.clone()
        }
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.cancel();
    }
    pub(crate) fn ensure_current(&self) -> Result<(), RouteError> {
        let now = self.snapshot.now_ms().map_err(|_| RouteError::Expired)?;
        if now >= self.expires_at_ms
            || Instant::now() >= self.deadline
            || self.cancelled.is_cancelled()
        {
            return Err(RouteError::Expired);
        }
        Ok(())
    }
    pub(crate) async fn invalidated(&self) {
        tokio::select! { biased;
            _ = self.snapshot.invalidated_at(self.expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.deadline.into()) => {},
            _ = self.cancelled.cancelled() => {},
        }
    }
}

fn policy_error(_: meshway_policy::dns_route::ResolverRouteError) -> RouteError {
    RouteError::Binding
}

/// Sealed exact local-business route, never a bootstrap socket exemption.
/// Adapters check this before every actual I/O poll and wait on `invalidated`.
#[derive(Clone)]
pub struct LocalResolverRoute {
    lifetime: QueryLifetime,
    endpoint: SocketAddr,
    transport: DnsTransport,
    action: LocalEgressAction,
    system: Option<crate::SystemResolverSnapshot>,
}

impl fmt::Debug for LocalResolverRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalResolverRoute").finish_non_exhaustive()
    }
}

/// A netmap's local decision for one resolver endpoint: the source egress
/// partition must leave it to a local socket. Exit-routed, Mesh and pool
/// addresses never get one.
fn netmap_local_action(
    netmap: &NodeNetmap,
    endpoint: SocketAddr,
) -> Result<LocalEgressAction, RouteError> {
    let pools = &netmap.self_.mesh_address_pools;
    if pools.ipv4.iter().chain(&pools.ipv6).any(|pool| {
        pool.parse::<ipnet::IpNet>()
            .is_ok_and(|net| net.contains(&endpoint.ip()))
    }) {
        return Err(RouteError::Binding);
    }
    let plan = netmap.egress.as_ref().ok_or(RouteError::Binding)?;
    match plan.route(endpoint.ip()).map(|route| &route.action) {
        Some(EgressAction::Direct) => Ok(LocalEgressAction::Direct),
        Some(EgressAction::Unmanaged) => Ok(LocalEgressAction::Unmanaged),
        _ => Err(RouteError::Binding),
    }
}

impl LocalResolverRoute {
    /// A configured resolver: the netmap's per-resolver decision
    /// (`dns_route::netmap_resolver`), which the source egress partition must
    /// leave to a local socket.
    pub(crate) fn new(
        snapshot: ExecutionSnapshot,
        key: &str,
        deadline: Instant,
    ) -> Result<Self, RouteError> {
        let reading = snapshot.netmap_reading().map_err(|_| RouteError::Expired)?;
        let netmap = &reading.netmap.netmap;
        let action =
            match meshway_policy::dns_route::netmap_resolver(netmap, key).map_err(policy_error)? {
                ResolverAction::Direct => LocalEgressAction::Direct,
                ResolverAction::Unmanaged => LocalEgressAction::Unmanaged,
                ResolverAction::Exit | ResolverAction::Rejected => return Err(RouteError::Binding),
            };
        let server = netmap
            .projection
            .configuration
            .dns
            .servers
            .get(key)
            .ok_or(RouteError::Binding)?;
        let endpoint = SocketAddr::new(server.server, server.server_port.get());
        let transport = server.transport;
        // The resolver decision and the source partition must agree.
        if netmap_local_action(netmap, endpoint)? != action {
            return Err(RouteError::Binding);
        }
        let expires_at_ms = reading.expires_at_ms;
        let route = Self {
            endpoint,
            transport,
            action,
            lifetime: QueryLifetime::new(snapshot, expires_at_ms, deadline),
            system: None,
        };
        route.ensure_current()?;
        Ok(route)
    }

    /// One OS resolver upstream. UDP and its TCP fallback are separate routes.
    pub(crate) fn system(
        snapshot: ExecutionSnapshot,
        system: crate::SystemResolverSnapshot,
        index: usize,
        transport: DnsTransport,
        deadline: Instant,
    ) -> Result<Self, RouteError> {
        system.ensure_current().map_err(|_| RouteError::Expired)?;
        let endpoint = *system.upstreams.get(index).ok_or(RouteError::Binding)?;
        if system.primary == DnsTransport::Tcp && transport != DnsTransport::Tcp {
            return Err(RouteError::Binding);
        }
        use meshway_configuration::netmap::DnsPlan;
        let reading = snapshot.netmap_reading().map_err(|_| RouteError::Expired)?;
        let netmap = &reading.netmap.netmap;
        // The resolver Node serves sources from its own OS resolvers too.
        if !netmap.self_.enabled
            || !matches!(
                netmap.dns.egress,
                DnsPlan::NoExit | DnsPlan::ServeExitResolver { .. }
            )
        {
            return Err(RouteError::Binding);
        }
        let action = netmap_local_action(netmap, endpoint)?;
        let expires_at_ms = reading.expires_at_ms;
        let route = Self {
            endpoint,
            transport,
            action,
            lifetime: QueryLifetime::new(snapshot, expires_at_ms, deadline),
            system: Some(system),
        };
        route.ensure_current()?;
        Ok(route)
    }

    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn transport(&self) -> DnsTransport {
        self.transport
    }
    pub fn action(&self) -> LocalEgressAction {
        self.action
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        self.lifetime.snapshot()
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.lifetime.expires_at_ms()
    }
    pub fn deadline(&self) -> Instant {
        self.lifetime.deadline()
    }

    pub(crate) fn bind_cancellation(&mut self, cancelled: CancellationToken) {
        self.lifetime.bind_cancellation(cancelled);
    }

    pub(crate) fn exchange_scope(&self, deadline: Instant) -> Self {
        Self {
            lifetime: self.lifetime.exchange(deadline),
            ..self.clone()
        }
    }

    pub(crate) fn cancel_exchange(&self) {
        self.lifetime.cancel();
    }

    pub fn ensure_current(&self) -> Result<(), RouteError> {
        if let Some(system) = &self.system {
            system.ensure_current().map_err(|_| RouteError::Expired)?;
        }
        self.lifetime.ensure_current()
    }

    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.lifetime.invalidated() => {},
            _ = async { match &self.system { Some(system) => system.invalidated().await, None => std::future::pending().await } } => {},
        }
    }
}
