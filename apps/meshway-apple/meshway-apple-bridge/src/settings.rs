//! Value conversion only; capture computation belongs to Apple Host.
use crate::BridgeError;
use std::{io, sync::Arc};

#[derive(uniffi::Record)]
pub struct Route {
    pub address: String,
    pub prefix: u8,
    pub netmask: String,
}
#[derive(uniffi::Record)]
pub struct NetworkSettings {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub routes4: Vec<Route>,
    pub routes6: Vec<Route>,
    pub mtu: u16,
}
#[uniffi::export(callback_interface)]
pub trait TunnelSettingsSink: Send + Sync {
    fn apply(&self, settings: Option<NetworkSettings>) -> Result<(), BridgeError>;
}
pub(super) struct Sink(pub Arc<dyn TunnelSettingsSink>);
impl meshway_node::host::apple::TunnelSettingsSink for Sink {
    fn apply(
        &self,
        settings: Option<meshway_node::host::apple::TunnelSettings>,
    ) -> meshway_node::host::SendFuture<'_, io::Result<()>> {
        let callback = self.0.clone();
        let settings = settings.map(|settings| {
            let (mut routes4, mut routes6) = (Vec::new(), Vec::new());
            for network in settings.routes {
                let route = Route {
                    address: network.network().to_string(),
                    prefix: network.prefix_len(),
                    netmask: network.netmask().to_string(),
                };
                if network.addr().is_ipv4() {
                    routes4.push(route);
                } else {
                    routes6.push(route);
                }
            }
            NetworkSettings {
                ipv4: settings.ipv4.map(|ip| ip.to_string()),
                ipv6: settings.ipv6.map(|ip| ip.to_string()),
                routes4,
                routes6,
                mtu: settings.mtu,
            }
        });
        Box::pin(async move {
            tokio::task::spawn_blocking(move || callback.apply(settings))
                .await
                .map_err(io::Error::other)?
                .map_err(io::Error::other)
        })
    }
}
