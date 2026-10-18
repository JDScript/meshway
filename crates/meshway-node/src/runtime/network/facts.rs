//! Project measured resources; readings do not create probes or RTT samples.
use super::Report;
use crate::settings::Settings;
use meshway_model::state::*;
use std::collections::BTreeMap;

impl Report {
    /// The State of this installation for the live instance. Only facts of
    /// the installed netmap are reported; `saved` is the held netmap.
    pub fn state(&self, control: &meshway_control::Control, settings: &Settings) -> NodeState {
        let current = self.current();
        let installed = self.netmap.filter(|_| current);
        let held = control.netmap_reference();
        let applied = installed.and_then(|_| self.applied());
        let instance = control.status().instance;
        let mut state = NodeState {
            system: settings.system.clone(),
            capabilities: super::capabilities(&settings.host_capabilities),
            configuration: ConfigurationState {
                saved: held,
                applied,
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            adjacencies: Default::default(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
            exit_capability: self
                .exit_capability
                .clone()
                .filter(|exit| {
                    installed == Some(exit.netmap)
                        && instance
                            .as_ref()
                            .is_some_and(|i| i.instance_id == exit.instance_id)
                })
                .map(|mut exit| {
                    if applied.is_none() {
                        exit.set_installed(false);
                    }
                    exit
                }),
            exit_dns: self
                .exit_dns
                .clone()
                .filter(|fact| installed == Some(fact.netmap)),
        };
        if !current {
            return state;
        }
        let Some(target) = installed else {
            return state;
        };
        let phase = match self.completion {
            Some(Ok(())) => ApplyPhase::Applied,
            Some(Err(_)) => ApplyPhase::Failed,
            None if self.plane == meshway_protocol::ipc::DataPlane::Failed => ApplyPhase::Failed,
            None => ApplyPhase::Applying,
        };
        let problem = self
            .completion
            .is_some_and(|result| result.is_err())
            .then_some(StateProblem::ConfigurationRejected {
                target,
                observed_at: self.observed_at,
            });
        state.configuration.progress = Some(ConfigurationProgress {
            target,
            phase,
            observed_at: self.observed_at,
            problem,
        });
        let Some(facts) = &self.facts else {
            return state;
        };
        for socket in &facts.sockets {
            if socket.reference.netmap != target {
                continue;
            }
            state.sockets.insert(
                socket.reference.socket_id.clone(),
                SocketState {
                    local_endpoints: None,
                    resource: socket.reference.resource.clone(),
                    netmap: Some(socket.reference.netmap),
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
                netmap: Some(target),
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
                    netmap: Some(target),
                },
            );
        }
        // A listener whose bind its owner retries: no socket, and why.
        for listener in &facts.unavailable {
            state
                .listeners
                .entry(listener.key.clone())
                .or_insert(ListenerState {
                    phase: ListenerPhase::Failed,
                    socket_ids: Vec::new(),
                    loaded_certificate_sha256: None,
                    problem: Some(StateProblem::ResourceUnavailable {
                        resource: ResourceRef::Listener {
                            key: listener.key.clone(),
                        },
                        observed_at: self.facts_observed_at,
                    }),
                    netmap: Some(target),
                });
        }
        let (Some(netmap), Some(lease_end)) = (&self.content, self.expires_at) else {
            return state;
        };
        let netmap = &netmap.netmap;
        // A link fact lasts until the lease ends, but never beyond the State
        // fact lifetime the Server accepts.
        let expiry = lease_end.min(
            self.facts_observed_at
                + chrono::Duration::seconds(meshway_protocol::STATE_FACT_MAX_SECONDS),
        );
        // Links and adjacency status describe applied content only; the
        // Server rejects them in a report that names none.
        if state.configuration.applied.is_none() {
            return state;
        }
        state.adjacencies = facts
            .adjacencies
            .iter()
            .filter(|(peer, _)| netmap.projection.configuration.nodes.contains_key(peer))
            .map(|(peer, status)| (*peer, *status))
            .collect();
        for link in &facts.links {
            let id = socket_key(&link.id);
            let socket_id = socket_key(&link.socket_id);
            let resource = if link.carrier.listener == netmap.node_id {
                ResourceRef::Listener {
                    key: link.carrier.listener_key.clone(),
                }
            } else {
                let Some((key, _)) = netmap.dialers.iter().find(|(_, dial)| {
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
                    netmap: Some(target),
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
        // A ready circuit: the endpoints completed the end-to-end handshake;
        // a relay forwarded it both ways.
        for (id, (nodes, role)) in &self.circuits {
            if nodes.iter().any(|node| {
                *node != netmap.node_id && !netmap.projection.configuration.nodes.contains_key(node)
            }) {
                continue;
            }
            state.paths.insert(
                socket_key(id),
                PathState {
                    nodes: nodes.clone(),
                    netmap: target,
                    reporter_role: *role,
                    evidence: if *role == ReporterRole::Relay {
                        PathEvidence::Sent
                    } else {
                        PathEvidence::PeerConfirmed
                    },
                    observed_at: self.facts_observed_at,
                    expires_at: expiry,
                },
            );
        }
        state
    }
}

fn socket_key(id: &[u8; 32]) -> String {
    id.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn socket_network(protocol: meshway_model::TransportProtocol) -> SocketNetwork {
    match protocol.outer_socket() {
        meshway_model::OuterSocket::Udp => SocketNetwork::Udp,
        meshway_model::OuterSocket::Tcp => SocketNetwork::Tcp,
    }
}
