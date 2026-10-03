use super::*;
use std::collections::BTreeSet;

impl DialParameters {
    pub fn transport(&self) -> TransportProtocol {
        match self {
            Self::NativeUdp { .. } => TransportProtocol::NativeUdp,
            Self::NativeTcp { .. } => TransportProtocol::NativeTcp,
            Self::Vless { .. } => TransportProtocol::Vless,
            Self::Trojan { .. } => TransportProtocol::Trojan,
        }
    }
}

impl NodeConfiguration {
    /// Probe-only projection of this immutable target. Never use these edges as
    /// business grants or end-to-end observation paths. Includes reverse dial
    /// candidates: either end may probe an authenticated bidirectional channel.
    pub fn neighbor_transports(&self) -> BTreeMap<NodeId, BTreeSet<String>> {
        let mut neighbors: BTreeMap<NodeId, BTreeSet<String>> = BTreeMap::new();
        for path in self.permits.values().flat_map(|permit| &permit.paths) {
            for pair in path.nodes.windows(2) {
                let peer = if pair[0] == self.node_id {
                    pair[1]
                } else if pair[1] == self.node_id {
                    pair[0]
                } else {
                    continue;
                };
                let allowed = neighbors.entry(peer).or_default();
                for key in path
                    .dial_candidates
                    .get(&self.node_id)
                    .into_iter()
                    .flatten()
                {
                    if let Some(candidate) = self
                        .dialers
                        .get(key)
                        .filter(|candidate| candidate.peer == peer)
                    {
                        allowed.insert(candidate.parameters.transport().as_str().into());
                    }
                }
                for (key, listener) in &self.listeners {
                    // These are compiler-owned candidate keys, not Node claims.
                    let candidate_key = format!("{}/{}", self.node_id, key);
                    if path
                        .dial_candidates
                        .get(&peer)
                        .is_some_and(|keys| keys.contains(&candidate_key))
                    {
                        let protocol = match listener.declaration() {
                            Listener::NativeUdp(_) => TransportProtocol::NativeUdp,
                            Listener::NativeTcp(_) => TransportProtocol::NativeTcp,
                            Listener::Vless(_) => TransportProtocol::Vless,
                            Listener::Trojan(_) => TransportProtocol::Trojan,
                        };
                        allowed.insert(protocol.as_str().into());
                    }
                }
            }
        }
        neighbors.retain(|_, transports| !transports.is_empty());
        neighbors
    }
}
