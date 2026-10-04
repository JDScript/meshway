use meshway_configuration::{DnsTransport, execution::LocalEgressAction};
use meshway_control::ExecutionSnapshot;
use meshway_model::RequestId;
use std::{fmt, net::SocketAddr, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RouteError {
    #[error("local resolver route is missing or does not match the publication")]
    Binding,
    #[error("local resolver route is expired or cancelled")]
    Expired,
}

/// Sealed exact local-business route, never a bootstrap socket exemption.
/// Adapters check this before every actual I/O poll and wait on `invalidated`.
#[derive(Clone)]
pub struct LocalResolverRoute {
    snapshot: ExecutionSnapshot,
    endpoint: SocketAddr,
    transport: DnsTransport,
    action: LocalEgressAction,
    demand: RequestId,
    expires_at_ms: u64,
    deadline: Instant,
    cancelled: tokio_util::sync::CancellationToken,
}

impl fmt::Debug for LocalResolverRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalResolverRoute").finish_non_exhaustive()
    }
}

impl LocalResolverRoute {
    pub(crate) fn new(
        snapshot: ExecutionSnapshot,
        key: &str,
        demand: RequestId,
        deadline: Instant,
    ) -> Result<Self, RouteError> {
        let now = snapshot.now_ms().map_err(|_| RouteError::Expired)?;
        let configuration = snapshot.configuration().map_err(|_| RouteError::Expired)?;
        let permit =
            meshway_policy::dns_route::local_resolver_route(configuration, key, demand, now)
                .map_err(|error| match error {
                    meshway_policy::dns_route::ResolverRouteError::Expired => RouteError::Expired,
                    _ => RouteError::Binding,
                })?;
        let server = configuration
            .dns
            .settings
            .servers
            .get(key)
            .ok_or(RouteError::Binding)?;
        let route = Self {
            endpoint: SocketAddr::new(server.server, server.server_port.get()),
            transport: server.transport,
            action: permit.action,
            demand,
            expires_at_ms: permit.expires_at_ms.min(snapshot.expires_at_ms()),
            deadline: deadline.min(snapshot.expires_at()),
            snapshot,
            cancelled: tokio_util::sync::CancellationToken::new(),
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
    pub fn demand_id(&self) -> RequestId {
        self.demand
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    pub(crate) fn bind_cancellation(&mut self, cancelled: tokio_util::sync::CancellationToken) {
        self.cancelled = cancelled;
    }

    pub fn ensure_current(&self) -> Result<(), RouteError> {
        let now = self.snapshot.now_ms().map_err(|_| RouteError::Expired)?;
        if now >= self.expires_at_ms
            || Instant::now() >= self.deadline
            || self.cancelled.is_cancelled()
        {
            return Err(RouteError::Expired);
        }
        Ok(())
    }

    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        tokio::select! { biased;
            _ = self.snapshot.invalidated_at(self.expires_at_ms) => {},
            _ = tokio::time::sleep_until(self.deadline.into()) => {},
            _ = self.cancelled.cancelled() => {},
        }
    }
}
