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
    base_url: String,
}

impl ControllerChannel {
    pub(crate) async fn prepare(
        reservation: Reservation,
        origin: reqwest::Url,
        upstreams: SystemDnsSnapshot,
    ) -> io::Result<Self> {
        let owner = PreparedGeneration::prepare(reservation).await?;
        Self::from_prepared(owner, origin, upstreams)
    }

    pub(crate) fn from_prepared(
        owner: PreparedGeneration,
        origin: reqwest::Url,
        upstreams: SystemDnsSnapshot,
    ) -> io::Result<Self> {
        // Generated operations concatenate a leading-slash API path. Url
        // canonicalizes a bare origin with '/', so normalize that delimiter.
        let base_url = origin.as_str().trim_end_matches('/').to_owned();
        let factory = owner.factory();
        let resolver = BootstrapResolver::new(factory.clone(), upstreams);
        let transport = Arc::new(ControllerHttp::new(
            factory,
            resolver,
            origin,
            ControllerHttp::system_roots()?,
        )?);
        Ok(Self {
            owner,
            transport,
            base_url,
        })
    }

    pub(crate) fn client(&self) -> HttpClient {
        // Retired channels remain injected and fail closed; no plain fallback.
        HttpClient::new()
            .with_base_url(&self.base_url)
            .with_transport(self.transport.clone())
    }

    pub(crate) fn retire(&self) {
        self.owner.retire();
    }
}
