//! Stable authorization identity; deadlines remain separately enforced grants.
use meshway_configuration::{SerializationError, digest, execution::PathPermit};
use meshway_model::Digest;
use serde::Serialize;

/// Hash every authorization field, excluding only the absolute permit expiry.
/// Exhaustive destructuring makes adding a permit field a compile-time review
/// requirement. No fabricated timestamp is passed through the wire serializer.
pub fn permit_authorization_digest(permit: &PathPermit) -> Result<Digest, SerializationError> {
    let PathPermit {
        demand_id,
        source,
        source_addresses,
        target,
        protocol,
        port,
        profile,
        paths,
        expires_at_ms: _,
        replies,
    } = permit;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Authorization<'a> {
        demand_id: &'a meshway_model::RequestId,
        source: &'a meshway_model::NodeId,
        source_addresses: &'a meshway_configuration::NodeAddresses,
        target: &'a meshway_configuration::execution::BusinessTarget,
        protocol: &'a meshway_configuration::execution::BusinessProtocol,
        port: &'a Option<meshway_transport_spec::Port>,
        profile: &'a str,
        paths: &'a [meshway_configuration::execution::LogicalPath],
        replies: &'a meshway_configuration::execution::ReplyPolicy,
    }
    digest(&Authorization {
        demand_id,
        source,
        source_addresses,
        target,
        protocol,
        port,
        profile,
        paths,
        replies,
    })
}
