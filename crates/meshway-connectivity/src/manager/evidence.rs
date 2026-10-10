//! Typed provenance of an actual authenticated channel installation. It is
//! captured once from the admitted scope, never reconstructed from later facts.
use super::{LinkFact, Scope};
use meshway_model::{BoundNodeKey, NetmapRef, NetworkId, NodeId, observations::ProbeChannel};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Installation<'a> {
    node: NodeId,
    network: NetworkId,
    instance: meshway_protocol::InstanceRequest,
    netmap: NetmapRef,
    expires_at_ms: u64,
    observed_at_ms: u64,
    control_key: &'a BoundNodeKey,
    dataplane_key: &'a BoundNodeKey,
    peer_control_key: &'a BoundNodeKey,
    peer_dataplane_key: &'a BoundNodeKey,
    carrier: &'a crate::Carrier,
    channel: ProbeChannel,
}

pub(super) fn installed(fact: &LinkFact, scope: &Scope) -> Option<String> {
    let snapshot = &scope.snapshot;
    let netmap = &snapshot.netmap().ok()?.netmap;
    let own = &netmap.self_.self_keys;
    let peer = netmap.projection.inventory.get(&fact.peer)?;
    let installation = Installation {
        node: netmap.node_id,
        network: netmap.network_id,
        instance: snapshot.instance()?,
        netmap: snapshot.reference(),
        expires_at_ms: snapshot.expires_at_ms(),
        observed_at_ms: snapshot.now_ms().ok()?,
        control_key: &own.control_key,
        dataplane_key: &own.dataplane_key,
        peer_control_key: &peer.control_key,
        peer_dataplane_key: &peer.dataplane_key,
        carrier: &fact.carrier,
        channel: ProbeChannel {
            link_id: fact.id.into(),
            socket_id: fact.socket_id.into(),
            peer_node_id: fact.peer,
            transport: fact.carrier.protocol,
            local_address: fact.local.try_into().ok()?,
            remote_address: fact.remote.try_into().ok()?,
            acquisition: fact.acquisition.clone(),
        },
    };
    serde_json::to_string(&installation).ok()
}
