//! Shared test I/O: injected sockets and the carriers of a netmap snapshot.
//! Real Server/Control identities come from [`super::netmap::support::Netmaps`].
use meshway_authority::DataPlaneIdentity;
use meshway_model::RequestId;
use std::collections::BTreeMap;

pub(super) fn request() -> RequestId {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    RequestId::new(uuid::Builder::from_random_bytes(bytes).into_uuid()).unwrap()
}

pub(in crate::manager) fn epoch(identity: &DataPlaneIdentity, generation: u64) -> super::Epoch {
    super::Epoch {
        identity: identity.clone(),
        plan: crate::plan::Plan::new(identity.snapshot()).unwrap(),
        generation: meshway_model::Sequence::new(generation).unwrap(),
    }
}

/// Native UDP dial carriers of the snapshot's own Node, by peer.
pub(in crate::manager) fn carriers(
    identity: &DataPlaneIdentity,
) -> BTreeMap<meshway_model::NodeId, crate::Carrier> {
    let netmap = &identity.snapshot().netmap().unwrap().netmap;
    netmap
        .dialers
        .values()
        .filter(|candidate| {
            candidate.parameters.transport() == meshway_transport_spec::TransportProtocol::NativeUdp
        })
        .map(|candidate| {
            (
                candidate.peer,
                crate::Carrier {
                    dialer: netmap.node_id,
                    listener: candidate.peer,
                    listener_key: candidate.listener_key.clone(),
                    protocol: candidate.parameters.transport(),
                },
            )
        })
        .collect()
}

mod sockets;
pub(super) use sockets::Factory;
