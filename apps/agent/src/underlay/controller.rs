use std::{io, sync::Arc};

use super::{
    PreparedGeneration, Reservation,
    dns::{BootstrapResolver, SystemDnsSnapshot},
    http::ControllerHttp,
};
use crate::generated::client::HttpClient;

/// Controller lifecycle, independent of business lease/sink retirement.
pub(crate) struct ControllerChannel {
    owner: PreparedGeneration,
    transport: Arc<ControllerHttp>,
}

impl ControllerChannel {
    pub(crate) async fn prepare(
        reservation: Reservation,
        origin: reqwest::Url,
        upstreams: SystemDnsSnapshot,
    ) -> io::Result<Self> {
        let owner = PreparedGeneration::prepare(reservation).await?;
        let factory = owner.factory();
        let resolver = BootstrapResolver::new(factory.clone(), upstreams);
        let transport = Arc::new(ControllerHttp::new(
            factory,
            resolver,
            origin,
            ControllerHttp::system_roots()?,
        )?);
        Ok(Self { owner, transport })
    }

    pub(crate) fn client(&self) -> HttpClient {
        // Retired channels remain injected and fail closed; no plain fallback.
        HttpClient::new().with_transport(self.transport.clone())
    }

    pub(crate) fn retire(&self) {
        self.owner.retire();
    }
}
