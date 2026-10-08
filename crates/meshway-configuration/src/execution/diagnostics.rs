use super::*;
use meshway_model::{InstanceId, ObservationId};

/// Exact authority referenced by a circuit. Diagnostics never impersonate a
/// business demand, even when their underlying UUID bytes happen to match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CircuitAuthorization {
    /// Business demand whose path permit authorizes the circuit.
    Business(RequestId),
    /// Observation whose independent diagnostic permit authorizes the circuit.
    Diagnostic(ObservationId),
}

tagged! {
/// Exact service scope checked by Access and routing when compiling an independent diagnostic permit.
pub enum ObservationServiceScope {
    /// Diagnose a path authorized for a specific TCP service.
    Tcp {
        /// TCP destination port used for service authorization.
        port: Port
    },
    /// Diagnose a path authorized for a specific UDP service.
    Udp {
        /// UDP destination port used for service authorization.
        port: Port
    },
    /// Diagnose a path authorized for ICMP echo without a transport port.
    IcmpEcho,
} }

object! {
/// Server-bound observation demand compiled separately from ordinary business traffic.
pub struct DiagnosticDemand {
    /// Shared observation task requiring diagnostic execution authority.
    pub id: ObservationId,
    /// Node authorized to originate the measurement.
    pub source: NodeId,
    /// Source runtime instance to which the observation is bound.
    pub instance_id: InstanceId,
    /// Node at the terminal of the measured path.
    pub target: NodeId,
    /// Exact service authorization scope to evaluate.
    pub service: ObservationServiceScope,
    /// Explicit complete ordered path, or null or omission to compile automatically selected candidates.
    pub path: Option<Vec<NodeId>>,
    /// Diagnostic demand deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }

object! {
/// Server-compiled short-lived measurement authority shared by all Nodes on its allowed complete paths.
pub struct DiagnosticPermit {
    /// Observation task authorized by this permit.
    pub observation_id: ObservationId,
    /// Only Node authorized to originate the diagnostic requests.
    pub source: NodeId,
    /// Exact source runtime instance authorized to execute the observation.
    pub instance_id: InstanceId,
    /// Terminal Node authorized to answer the diagnostic requests.
    pub target: NodeId,
    /// Exact authorized traffic counting scope; diagnostic frames are separate
    /// from TCP/UDP/ICMP business packets and create no business flow authority.
    pub service: ObservationServiceScope,
    /// Complete authorized measurement paths preserving every required intermediate Node.
    pub paths: Vec<LogicalPath>,
    /// Authentication and path constraint for diagnostic replies.
    pub replies: DiagnosticReplyPolicy,
    /// Permit deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; ordinary instance renewal does not extend it.
    #[serde(rename="expiresAt", with="meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type=meshway_model::timestamp::Timestamp],
} }

meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
/// Reply constraint for authenticated diagnostic request and response correlation.
pub enum DiagnosticReplyPolicy {
    /// Return authenticated replies along the exact reverse of the request's complete path.
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
