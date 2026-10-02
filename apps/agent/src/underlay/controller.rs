use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, RwLock},
};

use super::{
    PreparedGeneration, Reservation,
    dns::{BootstrapResolver, SystemDnsSnapshot},
    http::ControllerHttp,
};
use meshway_control_client::client::{HttpClient, HttpError, HttpResult, HttpTransport};

/// All generated consumer clones observe one replacement point. An empty or
/// retired slot fails closed, including already-created DNS consumers.
pub(crate) struct ControllerClients {
    transport: RwLock<Option<Arc<ControllerHttp>>>,
}
impl ControllerClients {
    pub(crate) fn new(channel: &ControllerChannel) -> Arc<Self> {
        Arc::new(Self {
            transport: RwLock::new(Some(channel.transport.clone())),
        })
    }
    pub(crate) fn install(&self, channel: &ControllerChannel) -> io::Result<()> {
        *self.transport.write().map_err(|_| super::closed())? = Some(channel.transport.clone());
        Ok(())
    }
    pub(crate) fn close(&self) {
        if let Ok(mut transport) = self.transport.write() {
            *transport = None;
        }
    }
}
impl HttpTransport for ControllerClients {
    fn execute(
        &self,
        request: reqwest::Request,
    ) -> Pin<Box<dyn Future<Output = HttpResult<reqwest::Response>> + Send + '_>> {
        let transport = self.transport.read().ok().and_then(|slot| slot.clone());
        Box::pin(async move {
            let transport = transport.ok_or_else(|| HttpError::Io(super::closed()))?;
            transport.execute(request).await
        })
    }
}

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

    pub(crate) fn active(&self) -> bool {
        self.owner.active()
    }

    pub(crate) async fn retire_and_join(self) {
        self.owner.retire();
        self.transport.join().await;
    }
}
