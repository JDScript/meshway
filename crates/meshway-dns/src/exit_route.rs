//! Sealed client-side Mesh resolver routes. The exchange adapter must use the
//! unique SecureNode and exact service purpose; a protected local socket is not
//! an implementation of this route.
use crate::{RouteError, route::QueryLifetime};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::{
    DnsTransport,
    execution::{DnsEgress, DnsServicePermit},
    netmap::{DnsPlan, DnsService},
};
use std::{fmt, net::SocketAddr, time::Instant};
use tokio_util::sync::CancellationToken;

/// The exact service this route carries; never interchangeable.
#[derive(Clone)]
enum Service {
    Legacy(Box<DnsServicePermit>),
    Netmap(Box<DnsService>),
}

#[derive(Clone)]
pub struct MeshResolverRoute {
    lifetime: QueryLifetime,
    service: Service,
    transport: DnsTransport,
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
        let policy = |error| match error {
            meshway_policy::dns_route::ResolverRouteError::Expired => RouteError::Expired,
            _ => RouteError::Binding,
        };
        let (service, expires_at_ms) = if snapshot.is_netmap() {
            let reading = snapshot.netmap_reading().map_err(|_| RouteError::Expired)?;
            let netmap = &reading.netmap.netmap;
            let DnsPlan::ViaExit { service } = &netmap.dns.egress else {
                return Err(RouteError::Binding);
            };
            meshway_policy::dns_route::netmap_dns_service(
                reading.leased(),
                service,
                netmap.node_id,
                service.exit,
                &service.path.nodes,
                transport,
                reading.now_ms,
            )
            .map_err(policy)?;
            (Service::Netmap(service.clone()), reading.expires_at_ms)
        } else {
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
            .map_err(policy)?;
            let expires_at_ms = service.expires_at_ms;
            (Service::Legacy(service.clone()), expires_at_ms)
        };
        let route = Self {
            lifetime: QueryLifetime::new(snapshot, expires_at_ms, deadline),
            service,
            transport,
        };
        route.ensure_current()?;
        Ok(route)
    }
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        self.lifetime.snapshot()
    }
    /// The legacy service permit; `None` under a netmap.
    pub fn permit(&self) -> Option<&DnsServicePermit> {
        match &self.service {
            Service::Legacy(permit) => Some(permit),
            Service::Netmap(_) => None,
        }
    }
    /// The netmap service; `None` under a legacy publication.
    pub fn service(&self) -> Option<&DnsService> {
        match &self.service {
            Service::Netmap(service) => Some(service),
            Service::Legacy(_) => None,
        }
    }
    pub fn endpoint(&self) -> SocketAddr {
        let address = match &self.service {
            Service::Legacy(permit) => permit.address.0,
            Service::Netmap(service) => service.address.0,
        };
        SocketAddr::new(address, 53)
    }
    pub fn transport(&self) -> DnsTransport {
        self.transport
    }
    pub fn deadline(&self) -> Instant {
        self.lifetime.deadline()
    }
    pub(crate) fn bind_cancellation(&mut self, token: CancellationToken) {
        self.lifetime.bind_cancellation(token);
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
        self.lifetime.ensure_current()
    }
    pub async fn invalidated(&self) {
        if self.ensure_current().is_err() {
            return;
        }
        self.lifetime.invalidated().await;
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
