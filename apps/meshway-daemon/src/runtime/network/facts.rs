//! Project measured resources; readings do not create probes or RTT samples.
use super::Report;
use crate::settings::Settings;
use meshway_model::state::*;
use std::collections::BTreeMap;

impl Report {
    pub fn state(&self, control: &meshway_control::Control, settings: &Settings) -> NodeState {
        let status = control.status();
        let current = self.current(&status);
        let applied = current.then(|| self.applied()).flatten();
        let saved = status.cached.filter(|saved| {
            status
                .instance
                .as_ref()
                .is_some_and(|i| i.target.as_ref() == Some(saved))
        });
        let mut state = NodeState {
            system: settings.system.clone(),
            capabilities: meshway_connectivity::capabilities(),
            configuration: ConfigurationState {
                saved,
                applied,
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
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
        for listener in &facts.listeners {
            let id = format!("listener:{}", listener.key);
            state.sockets.insert(
                id.clone(),
                SocketState {
                    resource: ResourceRef::Listener {
                        key: listener.key.clone(),
                    },
                    configuration: Some(target.clone()),
                    generation: self.generation,
                    network: SocketNetwork::Tcp,
                    bound_address: listener.local,
                    endpoints: Vec::new(),
                },
            );
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
            let id = link
                .id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
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
            state.sockets.insert(
                id.clone(),
                SocketState {
                    resource,
                    configuration: Some(target.clone()),
                    generation: self.generation,
                    network: SocketNetwork::Tcp,
                    bound_address: link.local,
                    endpoints: Vec::new(),
                },
            );
            state.links.entry(link.peer).or_default().push(LinkState {
                link_id: id.clone(),
                socket_id: id,
                transport: link.carrier.protocol.as_str().to_owned(),
                initiator_node_id: link.carrier.dialer,
                phase: LinkPhase::Authenticated,
                business_dependencies: self.dependencies.get(&link.peer).copied().unwrap_or(0),
                observation_ids: Vec::new(),
                observed_at: self.facts_observed_at,
                expires_at: expiry,
                since: None,
            });
        }
        state
    }
}
