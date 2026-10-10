//! Typed provenance of an actual authenticated channel installation. It is
//! captured once at installation from the applied content, never
//! reconstructed from later facts.
use super::{LinkFact, Shared};
use meshway_model::{BoundNodeKey, NetmapRef, NetworkId, NodeId, observations::ProbeChannel};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Installation<'a> {
    node: NodeId,
    network: NetworkId,
    instance: meshway_protocol::InstanceRequest,
    netmap: NetmapRef,
    observed_at_ms: u64,
    control_key: &'a BoundNodeKey,
    dataplane_key: &'a BoundNodeKey,
    peer_control_key: &'a BoundNodeKey,
    peer_dataplane_key: &'a BoundNodeKey,
    carrier: &'a crate::Carrier,
    channel: ProbeChannel,
}

pub(super) fn installed(fact: &LinkFact, shared: &Shared) -> Option<String> {
    let plan = shared.plan();
    let (peer_control_key, peer_dataplane_key) = plan.keys(fact.peer).ok()?;
    let installation = Installation {
        node: plan.node,
        network: plan.network,
        instance: shared.instance.lock().ok()?.clone()?,
        netmap: plan.reference,
        observed_at_ms: u64::try_from(chrono::Utc::now().timestamp_millis()).ok()?,
        control_key: &plan.self_keys.control_key,
        dataplane_key: &plan.self_keys.dataplane_key,
        peer_control_key,
        peer_dataplane_key,
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
