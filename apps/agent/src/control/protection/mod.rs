//! Controller ownership and durable real network binding; never business grants.
use crate::state::{
    Agent, AgentError, Identity, connections, dns, local,
    network_binding::{fingerprint, load_binding, persist_binding, sidecar},
};
use crate::{
    generated::{
        client::HttpClient,
        types::{AgentNodeResponse, EnrollAgentRequest, HeartbeatRequest},
    },
    underlay::{
        BootstrapChannel, BootstrapPreparation, BootstrapUnderlay, ControllerChannel,
        ControllerClients, LinuxUnderlay, Preparation, SystemDnsSnapshot,
    },
};

pub(crate) struct Control {
    pub(crate) owner: std::sync::Arc<tokio::sync::Mutex<LinuxUnderlay>>,
    channel: Option<ControllerChannel>,
    clients: std::sync::Arc<ControllerClients>,
    controller: String,
    dns: SystemDnsSnapshot,
}
impl Control {
    pub(crate) fn client(&self) -> HttpClient {
        HttpClient::new()
            .with_base_url(&self.controller)
            .with_transport(self.clients.clone())
    }
    pub(super) async fn refresh(&mut self) -> Result<(), AgentError> {
        let dns = match self.owner.lock().await.system_dns() {
            Ok(dns) => dns,
            Err(error) => {
                self.clients.close();
                if let Some(channel) = &self.channel {
                    channel.retire();
                }
                return Err(error.into());
            }
        };
        if self.dns == dns && self.channel.as_ref().is_some_and(ControllerChannel::active) {
            return Ok(());
        }
        let owner = self.owner.lock().await;
        let generation = owner.socket_generation().await?;
        let channel = ControllerChannel::from_prepared(
            generation,
            reqwest::Url::parse(&self.controller)
                .map_err(|_| AgentError::Invalid("invalid controller binding"))?,
            dns.clone(),
        )?;
        drop(owner);
        self.clients.install(&channel)?;
        if let Some(old) = self.channel.replace(channel) {
            old.retire_and_join().await;
        }
        self.dns = dns;
        Ok(())
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.clients.close();
        if let Some(channel) = &self.channel {
            channel.retire();
        }
    }
}
pub(crate) struct Bootstrap {
    owner: BootstrapUnderlay,
    channel: Option<BootstrapChannel>,
}
impl Agent {
    /// Join Controller drivers before immediately reusing the same identity in
    /// this process. Dropping an Agent fences I/O but cannot synchronously join.
    pub async fn close_control_plane(&mut self) {
        if let Some(mut control) = self.underlay.take() {
            control.clients.close();
            if let Some(channel) = control.channel.take() {
                channel.retire_and_join().await;
            }
        }
        if let Some(mut bootstrap) = self.bootstrap.take()
            && let Some(channel) = bootstrap.channel.take()
        {
            channel.retire().await;
        }
    }
    pub(crate) async fn prepare_underlay(
        &mut self,
        enrollment: Option<&str>,
    ) -> Result<Option<AgentNodeResponse>, AgentError> {
        #[cfg(test)]
        if !self.protect_underlay {
            return Ok(None);
        }
        if let Some(control) = &mut self.underlay {
            control.refresh().await?;
            return Ok(None);
        }
        local::ensure_durable(&self.path)?;
        if Identity::load(&self.path)? != self.identity {
            return Err(AgentError::Invalid(
                "identity changed before protected startup",
            ));
        }
        let directory = sidecar(&self.path, ".underlay");
        local::private_dir(&directory)?;
        let binding = load_binding(self)?;
        let mut accepted = None;
        if binding.is_none() || directory.join("bootstrap.json").try_exists()? {
            if self.bootstrap.is_none() {
                let fingerprint = fingerprint(&self.identity)?;
                let owner = BootstrapUnderlay::prepare(BootstrapPreparation {
                    state_directory: &directory,
                    fingerprint: &fingerprint,
                })
                .await?;
                self.bootstrap = Some(Bootstrap {
                    owner,
                    channel: None,
                });
            }
            if binding.is_none() {
                if self
                    .bootstrap
                    .as_ref()
                    .expect("bootstrap owner")
                    .channel
                    .is_none()
                {
                    let channel = self
                        .bootstrap
                        .as_ref()
                        .expect("bootstrap owner")
                        .owner
                        .channel(
                            reqwest::Url::parse(&self.identity.controller)
                                .map_err(|_| AgentError::Invalid("invalid persisted controller"))?,
                        )
                        .await?;
                    self.bootstrap.as_mut().expect("bootstrap owner").channel = Some(channel);
                }
                let channel = self
                    .bootstrap
                    .as_mut()
                    .expect("bootstrap owner")
                    .channel
                    .as_mut()
                    .expect("bootstrap channel");
                let response = if let Some(token) = enrollment {
                    channel
                        .enroll(
                            token,
                            EnrollAgentRequest {
                                name: self.identity.name.clone(),
                                agent_version: self.identity.enrollment_version.clone(),
                                node_credential: self.identity.node_credential.clone(),
                            },
                        )
                        .await?
                } else {
                    channel
                        .heartbeat(
                            &self.identity.node_credential,
                            HeartbeatRequest {
                                agent_version: env!("CARGO_PKG_VERSION").into(),
                            },
                        )
                        .await?
                };
                accepted = Some(self.accept(response)?);
                let wire = self
                    .bootstrap
                    .as_mut()
                    .expect("bootstrap owner")
                    .channel
                    .as_mut()
                    .expect("bootstrap channel")
                    .scope(&self.identity.node_credential)
                    .await?;
                let snapshot: meshway_protocol::AgentDnsSnapshot = connections::convert(wire)?;
                let old = dns::cached_scope(self)?;
                dns::validate_scope(
                    &snapshot,
                    self.identity.node_id.as_deref().expect("accepted node"),
                    old.as_ref(),
                )?;
                let network = &snapshot.payload.network_id;
                for previous in connections::historical_networks(self)? {
                    if &previous != network {
                        return Err(AgentError::Invalid(
                            "bootstrap network conflicts with saved binding",
                        ));
                    }
                }
                persist_binding(self, network)?;
            }
        }
        let binding =
            load_binding(self)?.ok_or(AgentError::Invalid("missing durable network binding"))?;
        let dns = dns::cached_scope(self)?;
        let mut historical = connections::historical_networks(self)?;
        if let Some(dns) = dns {
            historical.push(dns.payload.network_id);
        }
        if historical
            .iter()
            .any(|network| network != &binding.network_id)
        {
            return Err(AgentError::Invalid(
                "saved network binding conflicts with historical scope",
            ));
        }
        let input = Preparation {
            state_directory: &directory,
            identity: &binding.node_id,
            network: &binding.network_id,
        };
        let owner = if let Some(mut bootstrap) = self.bootstrap.take() {
            if let Some(channel) = bootstrap.channel.take() {
                channel.retire().await;
            }
            bootstrap.owner.bind(input).await?
        } else {
            LinuxUnderlay::prepare(input).await?
        };
        let channel = owner
            .controller_channel(
                reqwest::Url::parse(&binding.controller)
                    .map_err(|_| AgentError::Invalid("invalid bound controller"))?,
            )
            .await?;
        let dns = owner.system_dns()?;
        let clients = ControllerClients::new(&channel);
        self.underlay = Some(Control {
            owner: std::sync::Arc::new(tokio::sync::Mutex::new(owner)),
            channel: Some(channel),
            clients,
            controller: binding.controller,
            dns,
        });
        Ok(accepted)
    }
}

#[cfg(test)]
mod tests;
