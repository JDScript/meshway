use super::*;
use meshway_model::{InstanceId, ObservationId};

/// Exact authority referenced by a circuit. Diagnostics never impersonate a
/// business demand, even when their underlying UUID bytes happen to match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CircuitAuthorization {
    Business(RequestId),
    Diagnostic(ObservationId),
}

tagged! { pub enum ObservationServiceScope {
    Tcp { port: Port },
    Udp { port: Port },
    IcmpEcho,
} }

object! { pub struct DiagnosticDemand {
    pub id: ObservationId,
    pub source: NodeId,
    pub instance_id: InstanceId,
    pub target: NodeId,
    pub service: ObservationServiceScope,
    pub path: Option<Vec<NodeId>>,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }

object! { pub struct DiagnosticPermit {
    pub observation_id: ObservationId,
    pub source: NodeId,
    pub instance_id: InstanceId,
    pub target: NodeId,
    /// Exact authorized traffic counting scope; diagnostic frames are separate
    /// from TCP/UDP/ICMP business packets and create no business flow authority.
    pub service: ObservationServiceScope,
    pub paths: Vec<LogicalPath>,
    pub replies: DiagnosticReplyPolicy,
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }

meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReplyPolicy {
    AuthenticatedExactReversePath,
}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_and_diagnostic_authority_remain_distinct_for_the_same_uuid() {
        let id = "00000000-0000-4000-8000-000000000001";
        let business = CircuitAuthorization::Business(id.parse().unwrap());
        let diagnostic = CircuitAuthorization::Diagnostic(id.parse().unwrap());
        assert_ne!(business, diagnostic);
        let authorities = std::collections::BTreeSet::from([business, diagnostic]);
        assert_eq!(authorities.len(), 2);
    }
}
