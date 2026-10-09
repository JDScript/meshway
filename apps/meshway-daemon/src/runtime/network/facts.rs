//! Project measured resources; readings do not create probes or RTT samples.
use super::Report;
use crate::settings::Settings;
use meshway_model::state::*;
use std::collections::BTreeMap;

impl Report {
    pub fn state(&self, control: &meshway_control::Control, settings: &Settings) -> NodeState {
        let status = control.status();
        let current = self.current(&status);
        if !current {
            tracing::debug!(
                instance_matches = self.same_instance(&status),
                target_matches = self.bound_to(&status),
                execution_current = self.guard.as_ref().is_none_or(|guard| guard.ensure_current().is_ok()),
                dns_current = self.dns_guard.as_ref().is_none_or(|guard| guard.has_changed().is_ok() && guard.borrow().is_none()),
                report_publication = ?self.ticket.as_ref().map(|ticket| ticket.target().execution_publication),
                control_publication = ?status.instance.as_ref().and_then(|instance| instance.target.as_ref().map(|target| target.execution_publication)),
                "Runtime State sampled outside current installation"
            );
        }
        let applied = current.then(|| self.applied()).flatten();
        let saved = status.cached.clone().filter(|saved| {
            status
                .instance
                .as_ref()
                .is_some_and(|i| i.target.as_ref() == Some(saved))
        });
        let mut state = NodeState {
            system: settings.system.clone(),
            capabilities: super::capabilities(),
            configuration: ConfigurationState {
                saved,
                applied: applied.clone(),
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
            exit_capability: self
                .exit_capability
                .clone()
                .filter(|exit| {
                    self.bound_to(&status)
                        && status.instance.as_ref().is_some_and(|i| {
                            i.instance_id == exit.instance_id
                                && i.target.as_ref() == Some(&exit.target)
                        })
                })
                .map(|mut exit| {
                    if applied.is_none() {
                        exit.installed_families.clear();
                    }
                    exit
                }),
        };
        if !current {
            return state;
        }
        let Some(target) = self.ticket.as_ref().map(|ticket| ticket.target()) else {
            return state;
        };
        let phase = match self.completion {
            Some(Ok(())) => ApplyPhase::Applied,
            Some(Err(_)) => ApplyPhase::Failed,
            None if self.plane == meshway_ipc::DataPlane::Failed => ApplyPhase::Failed,
            None => ApplyPhase::Applying,
        };
        let problem = self
            .completion
            .is_some_and(|result| result.is_err())
            .then(|| StateProblem::ConfigurationRejected {
                target: target.clone(),
                observed_at: self.observed_at,
            });
        state.configuration.progress = Some(ConfigurationProgress {
            target: target.clone(),
            phase,
            observed_at: self.observed_at,
            problem,
        });
        let Some(facts) = &self.facts else {
            return state;
        };
        for socket in &facts.sockets {
            if socket.reference.configuration != *target {
                continue;
            }
            state.sockets.insert(
                socket.reference.socket_id.clone(),
                SocketState {
                    local_endpoints: None,
                    resource: socket.reference.resource.clone(),
                    configuration: Some(socket.reference.configuration.clone()),
                    generation: socket.reference.socket_generation,
                    network: socket.network,
                    bound_address: socket.bound_address,
                    endpoints: socket.endpoints.clone(),
                },
            );
        }
        for listener in &facts.listeners {
            let id = socket_key(&listener.socket_id);
            state.sockets.entry(id.clone()).or_insert(SocketState {
                local_endpoints: None,
                resource: ResourceRef::Listener {
                    key: listener.key.clone(),
                },
                configuration: Some(target.clone()),
                generation: self.generation,
                network: socket_network(listener.protocol),
                bound_address: listener.local,
                endpoints: Vec::new(),
            });
            state.listeners.insert(
                listener.key.clone(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec![id],
                    loaded_certificate_sha256: None,
                    problem: None,
                    configuration: Some(target.clone()),
                },
            );
        }
        let Some(guard) = &self.guard else {
            return state;
        };
        let Ok(cfg) = guard.configuration() else {
            return state;
        };
        let Some(expiry) = i64::try_from(guard.expires_at_ms())
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
        else {
            return state;
        };
        for link in &facts.links {
            let id = socket_key(&link.id);
            let socket_id = socket_key(&link.socket_id);
            let resource = if link.carrier.listener == cfg.node_id {
                ResourceRef::Listener {
                    key: link.carrier.listener_key.clone(),
                }
            } else {
                let Some((key, _)) = cfg.dialers.iter().find(|(_, dial)| {
                    dial.peer == link.peer && dial.listener_key == link.carrier.listener_key
                }) else {
                    continue;
                };
                ResourceRef::Dial { key: key.clone() }
            };
            state
                .sockets
                .entry(socket_id.clone())
                .or_insert(SocketState {
                    local_endpoints: None,
                    resource,
                    configuration: Some(target.clone()),
                    generation: self.generation,
                    network: socket_network(link.carrier.protocol),
                    bound_address: link.local,
                    endpoints: Vec::new(),
                });
            state.links.entry(link.peer).or_default().push(LinkState {
                acquisition: Some(link.acquisition.clone()),
                link_id: id.clone(),
                socket_id,
                transport: link.carrier.protocol.as_str().to_owned(),
                initiator_node_id: link.carrier.dialer,
                phase: LinkPhase::Authenticated,
                business_dependencies: self.dependencies.get(&link.peer).copied().unwrap_or(0),
                observation_ids: self
                    .observation_dependencies
                    .get(&link.peer)
                    .cloned()
                    .unwrap_or_default(),
                observed_at: self.facts_observed_at,
                expires_at: expiry,
                since: None,
            });
        }
        state
    }
}

fn socket_key(id: &[u8; 32]) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn socket_network(protocol: meshway_configuration::transport::TransportProtocol) -> SocketNetwork {
    match protocol.outer_socket() {
        meshway_configuration::transport::OuterSocket::Udp => SocketNetwork::Udp,
        meshway_configuration::transport::OuterSocket::Tcp => SocketNetwork::Tcp,
    }
}
