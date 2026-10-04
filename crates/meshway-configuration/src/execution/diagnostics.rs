use super::*;
use meshway_model::{InstanceId, ObservationId};

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
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
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
    pub expires_at_ms: u64 [value_type=chrono::DateTime<chrono::Utc>],
} }

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReplyPolicy {
    AuthenticatedExactReversePath,
}
