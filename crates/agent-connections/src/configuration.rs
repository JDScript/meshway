use crate::{Failure, Reason};
use meshway_dataplane::{
    LabConfig, ListenerConfig, OutgoingCandidate, PeerConfig, PrivateKey, TransportKind,
};
use meshway_protocol::{ConnectionTransport, NodeConnections};
use std::{collections::BTreeMap, collections::BTreeSet, net::SocketAddr};

fn transport(value: ConnectionTransport) -> TransportKind {
    match value {
        ConnectionTransport::NativeUdp => TransportKind::NativeUdp,
        ConnectionTransport::NativeTcp => TransportKind::NativeTcp,
        ConnectionTransport::Vless => TransportKind::Vless,
    }
}
pub(super) fn protocol_transport(value: TransportKind) -> ConnectionTransport {
    match value {
        TransportKind::NativeUdp => ConnectionTransport::NativeUdp,
        TransportKind::NativeTcp => ConnectionTransport::NativeTcp,
        TransportKind::Vless => ConnectionTransport::Vless,
    }
}
pub(super) fn allowed_peers(intent: &NodeConnections, listener: &str) -> Vec<String> {
    intent
        .incoming
        .iter()
        .filter(|approval| approval.listener_id == listener)
        .map(|approval| approval.peer_node_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub(super) fn candidate_label(id: &str, index: usize) -> String {
    if index == 0 {
        id.to_owned()
    } else {
        format!("{id}:address:{index}")
    }
}
pub fn controller_candidate<'a>(
    intent: &'a NodeConnections,
    label: &str,
) -> Option<&'a meshway_protocol::OutgoingConnectionCandidate> {
    intent
        .outgoing
        .iter()
        .find(|candidate| (0..8).any(|index| candidate_label(&candidate.id, index) == label))
}
pub fn core_config(
    intent: &NodeConnections,
    private_key: PrivateKey,
    targets: &BTreeMap<String, Vec<SocketAddr>>,
) -> Result<LabConfig, Failure> {
    let trusted_peers = intent
        .peers
        .iter()
        .map(|peer| {
            let transports = intent
                .outgoing
                .iter()
                .filter(|item| item.peer_node_id == peer.node_id)
                .map(|item| transport(item.transport))
                .chain(
                    intent
                        .incoming
                        .iter()
                        .filter(|item| item.peer_node_id == peer.node_id)
                        .map(|item| transport(item.transport)),
                )
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            PeerConfig {
                node_id: peer.node_id.clone(),
                public_key: peer.public_key.clone(),
                transports,
            }
        })
        .collect();
    let listeners = intent
        .listeners
        .iter()
        .filter(|item| item.transport != ConnectionTransport::Vless)
        .map(|item| {
            Ok(ListenerConfig {
                transport: transport(item.transport),
                bind: item
                    .bind
                    .parse()
                    .map_err(|_| Failure(Reason::InvalidConfiguration))?,
                id: Some(item.id.clone()),
                allowed_peers: Some(allowed_peers(intent, &item.id)),
            })
        })
        .collect::<Result<Vec<_>, Failure>>()?;
    let mut outgoing = Vec::new();
    let mut native_targets = BTreeSet::new();
    for item in intent
        .outgoing
        .iter()
        .filter(|item| item.transport != ConnectionTransport::Vless)
    {
        let addresses = targets
            .get(&item.id)
            .filter(|targets| !targets.is_empty())
            .ok_or(Failure(Reason::ResolveFailed))?;
        for (index, address) in addresses.iter().enumerate() {
            if !native_targets.insert((&item.peer_node_id, transport(item.transport), *address)) {
                continue;
            }
            outgoing.push(OutgoingCandidate {
                peer: item.peer_node_id.clone(),
                transport: transport(item.transport),
                endpoint: address.to_string(),
                priority: item.priority,
                id: Some(candidate_label(&item.id, index)),
            });
        }
    }
    let config = LabConfig {
        node_id: intent.node_id.clone(),
        private_key,
        public_key: intent
            .public_key
            .clone()
            .ok_or(Failure(Reason::InvalidConfiguration))?,
        trusted_peers,
        allowed_paths: Vec::new(),
        listeners,
        outgoing,
        limits: Default::default(),
    };
    config
        .validate()
        .map_err(|_| Failure(Reason::InvalidConfiguration))?;
    Ok(config)
}
