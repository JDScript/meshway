//! Stable authorization identity; deadlines remain separately enforced grants.
use meshway_configuration::{SerializationError, digest, execution::PathPermit};
use meshway_model::Digest;
use serde::Serialize;

/// A continuity candidate still needs Control's live snapshot/instance checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermitTransition {
    Renewal,
    Replacement,
}

pub fn permit_transition(
    previous: &PathPermit,
    next: &PathPermit,
    now_ms: u64,
) -> Result<PermitTransition, SerializationError> {
    Ok(
        if previous.expires_at_ms > now_ms
            && next.expires_at_ms > now_ms
            && permit_authorization_digest(previous)? == permit_authorization_digest(next)?
        {
            PermitTransition::Renewal
        } else {
            PermitTransition::Replacement
        },
    )
}

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
        exit,
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
        exit: &'a Option<meshway_configuration::execution::ExitPermit>,
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
        exit,
        paths,
        replies,
    })
}

/// Domain-separated diagnostic authority, excluding only its renewable expiry.
pub fn diagnostic_authorization_digest(
    permit: &meshway_configuration::execution::DiagnosticPermit,
) -> Result<Digest, SerializationError> {
    use meshway_configuration::execution::*;
    let DiagnosticPermit {
        observation_id,
        source,
        instance_id,
        target,
        service,
        paths,
        replies,
        expires_at_ms: _,
    } = permit;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Authorization<'a> {
        domain: &'static str,
        observation_id: &'a meshway_model::ObservationId,
        source: &'a meshway_model::NodeId,
        instance_id: &'a meshway_model::InstanceId,
        target: &'a meshway_model::NodeId,
        service: &'a ObservationServiceScope,
        paths: &'a [LogicalPath],
        replies: &'a DiagnosticReplyPolicy,
    }
    digest(&Authorization {
        domain: "meshway/diagnostic-authorization/v3",
        observation_id,
        source,
        instance_id,
        target,
        service,
        paths,
        replies,
    })
}
