//! Sealed client-side Mesh resolver routes. The exchange adapter must use the
//! unique SecureNode and exact service purpose; a protected local socket is not
//! an implementation of this route.
use crate::RouteError;
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::{
    DnsTransport,
    execution::{DnsEgress, DnsServicePermit},
};
use std::{fmt, net::SocketAddr, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct MeshResolverRoute {
    snapshot: ExecutionSnapshot,
    permit: DnsServicePermit,
    transport: DnsTransport,
    deadline: Instant,
    cancelled: CancellationToken,
}

impl fmt::Debug for MeshResolverRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeshResolverRoute").finish_non_exhaustive()
    }
}

impl MeshResolverRoute {
    pub(crate) fn new(
        snapshot: ExecutionSnapshot,
        transport: DnsTransport,
        deadline: Instant,
    ) -> Result<Self, RouteError> {
        let cfg = snapshot.configuration().map_err(|_| RouteError::Expired)?;
        let DnsEgress::ViaExit { service, .. } = &cfg.dns.egress else {
            return Err(RouteError::Binding);
        };
        if service.source_instance_id != snapshot.instance().instance_id {
            return Err(RouteError::Binding);
        }
        let now = snapshot.now_ms().map_err(|_| RouteError::Expired)?;
        meshway_policy::dns_route::dns_service_permit(
            cfg,
            service,
            cfg.node_id,
            service.exit,
            &service.path.nodes,
            transport,
            now,
        )
        .map_err(|error| match error {
            meshway_policy::dns_route::ResolverRouteError::Expired => RouteError::Expired,
            _ => RouteError::Binding,
        })?;
        let route = Self {
            permit: service.as_ref().clone(),
            transport,
            deadline: deadline.min(snapshot.expires_at()),
            snapshot,
            cancelled: CancellationToken::new(),
        };
        route.ensure_current()?;
        Ok(route)
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }
    pub fn permit(&self) -> &DnsServicePermit {
        &self.permit
    }
    pub fn endpoint(&self) -> SocketAddr {
        SocketAddr::new(self.permit.address.0, 53)
    }
    pub fn transport(&self) -> DnsTransport {
        self.transport
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn bind_cancellation(&mut self, token: CancellationToken) {
        self.cancelled = token;
    }
    pub(crate) fn exchange_scope(&self, deadline: Instant) -> Self {
        Self {
            deadline: self.deadline.min(deadline),
            cancelled: self.cancelled.child_token(),
            ..self.clone()
        }
    }
    pub(crate) fn cancel_exchange(&self) {
        self.cancelled.cancel();
    }
    pub fn ensure_current(&self) -> Result<(), RouteError> {
        let now = self.snapshot.now_ms().map_err(|_| RouteError::Expired)?;
        if now >= self.permit.expires_at_ms
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
            _ = self.snapshot.invalidated_at(self.permit.expires_at_ms) => {},
            _ = self.cancelled.cancelled() => {},
            _ = tokio::time::sleep_until(self.deadline.into()) => {},
        }
    }
}

/// Distinct route purposes share ticket/response lifetime handling, never socket
/// authority. In particular Mesh must not be passed to the local exchange API.
#[derive(Clone, Debug)]
pub enum ResolverRoute {
    Local(crate::LocalResolverRoute),
    Mesh(MeshResolverRoute),
}
impl ResolverRoute {
    pub fn transport(&self) -> DnsTransport {
        match self {
            Self::Local(r) => r.transport(),
            Self::Mesh(r) => r.transport(),
        }
    }
    pub fn ensure_current(&self) -> Result<(), RouteError> {
        match self {
            Self::Local(r) => r.ensure_current(),
            Self::Mesh(r) => r.ensure_current(),
        }
    }
    pub async fn invalidated(&self) {
        match self {
            Self::Local(r) => r.invalidated().await,
            Self::Mesh(r) => r.invalidated().await,
        }
    }
    pub(crate) fn exchange_scope(&self, deadline: Instant) -> Self {
        match self {
            Self::Local(r) => Self::Local(r.exchange_scope(deadline)),
            Self::Mesh(r) => Self::Mesh(r.exchange_scope(deadline)),
        }
    }
    pub(crate) fn cancel_exchange(&self) {
        match self {
            Self::Local(r) => r.cancel_exchange(),
            Self::Mesh(r) => r.cancel_exchange(),
        }
    }
}
