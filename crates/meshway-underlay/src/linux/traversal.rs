//! Linux resource lifetime attachment to the common traversal authorization.
use super::{ProtectedDatagram, dns::Resolver, protection::ProtectionScope};
pub(super) use crate::traversal::{Registry, references};
use crate::{
    Result,
    traversal::{Context, DatagramAuthorization},
};
use std::{net::SocketAddr, sync::Arc};

pub struct TraversalDatagram {
    pub endpoint: ProtectedDatagram,
    pub authorization: DatagramAuthorization,
}

struct Attachment {
    scope: ProtectionScope,
    resolver: Resolver,
}
impl Context for Attachment {
    fn execution(&self) -> Result<meshway_engine::Desired> {
        self.scope.check()?;
        self.scope.execution()
    }
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<SocketAddr>>> {
        Box::pin(async move {
            let resolver = self.resolver.scoped(self.scope.shared.installed_data()?)?;
            resolver.resolve(host, port).await
        })
    }
}

impl DatagramAuthorization {
    pub(super) fn attach(
        mut endpoint: ProtectedDatagram,
        resolver: Resolver,
        reference: meshway_control::connectivity::SocketReference,
    ) -> Result<TraversalDatagram> {
        let local = endpoint.local_addr()?;
        let context = Arc::new(Attachment {
            scope: endpoint.scope(),
            resolver,
        });
        let (authorization, registry) = Self::new(context, reference, local);
        endpoint.traversal = Some(registry);
        Ok(TraversalDatagram {
            endpoint,
            authorization,
        })
    }
}
