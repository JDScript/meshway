//! Shared test I/O: injected sockets and the carriers of a desired netmap.
//! Real Server/Control identities come from [`super::netmap::support::Netmaps`].
use meshway_engine::Desired;
use std::collections::BTreeMap;

/// Native UDP dial carriers of the content's own Node, by peer.
pub(in crate::manager) fn carriers(
    desired: &Desired,
) -> BTreeMap<meshway_model::NodeId, crate::Carrier> {
    let netmap = desired.netmap();
    netmap
        .dialers
        .values()
        .filter(|candidate| {
            candidate.parameters.transport() == meshway_model::TransportProtocol::NativeUdp
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
