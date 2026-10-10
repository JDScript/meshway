//! Pure circuit and packet predicates. Only the installed netmap and its
//! owner-issued lease authorize execution; replies also need local flow state.
use crate::compile::MAX_HOPS;
pub use meshway_configuration::execution::Carrier;
use meshway_configuration::{
    NodeAddresses,
    execution::{BusinessProtocol, LogicalPath},
};
use meshway_model::{BoundNodeKey, KeyAlgorithm, NodeId};
use meshway_transport_spec::Port;
use std::net::IpAddr;
pub mod netmap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Traffic {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub protocol: BusinessProtocol,
    pub source_port: Option<Port>,
    pub destination_port: Option<Port>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Hop {
    Forward(NodeId),
    Deliver,
    Exit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AdmissionError {
    #[error("invalid or disabled configuration")]
    Configuration,
    #[error("permit is absent or inconsistent")]
    Permit,
    #[error("complete path is not permitted")]
    Path,
    #[error("arrival peer is not permitted")]
    Peer,
    #[error("traffic is not permitted")]
    Traffic,
    #[error("external exit is disabled")]
    ExitDisabled,
}
fn contains(addresses: &NodeAddresses, address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => addresses.ipv4 == Some(address),
        IpAddr::V6(address) => addresses.ipv6 == Some(address),
    }
}
fn keys_valid(control: &BoundNodeKey, dataplane: &BoundNodeKey) -> bool {
    control.public_key.algorithm == KeyAlgorithm::Ed25519
        && dataplane.public_key.algorithm == KeyAlgorithm::X25519
        && control.id != dataplane.id
}
fn valid_traffic(traffic: &Traffic) -> bool {
    traffic.source.is_ipv4() == traffic.destination.is_ipv4()
        && !traffic.source.is_unspecified()
        && !traffic.destination.is_unspecified()
        && !traffic.source.is_multicast()
        && !traffic.destination.is_multicast()
        && match traffic.protocol {
            BusinessProtocol::Tcp | BusinessProtocol::Udp => {
                traffic.source_port.is_some() && traffic.destination_port.is_some()
            }
            BusinessProtocol::IcmpEcho => {
                traffic.source_port.is_none() && traffic.destination_port.is_none()
            }
        }
}
fn valid_path(path: &[NodeId]) -> bool {
    (2..=MAX_HOPS).contains(&path.len())
        && !path
            .iter()
            .enumerate()
            .any(|(index, node)| path[..index].contains(node))
}
/// Check actual dial orientation on an edge of the complete selected path.
/// Both endpoints additionally check adjacency declarations and live authority.
pub fn path_carrier(path: &LogicalPath, carrier: &Carrier) -> bool {
    valid_path(&path.nodes)
        && !carrier.listener_key.is_empty()
        && path.nodes.windows(2).any(|edge| {
            (edge[0] == carrier.dialer && edge[1] == carrier.listener)
                || (edge[1] == carrier.dialer && edge[0] == carrier.listener)
        })
        && path
            .dial_candidates
            .get(&carrier.dialer)
            .is_some_and(|keys| {
                keys.contains(&format!("{}/{}", carrier.listener, carrier.listener_key))
            })
}
