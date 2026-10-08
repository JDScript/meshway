use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, PublicationId, RequestId, Revision};

pub use meshway_model::resources::RuleSetData;
wire_object! { #[derive(Clone, Debug)]
/// Transient validated rule-set content summary produced by a noncommitting configuration preview.
pub struct PreparedRuleSet {
    /// Digest of the complete normalized destination content, without reserving an artifact ID.
    pub digest: Digest,
    /// Total normalized CIDR, suffix and exact-name entries.
    pub entries: u32,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Immutable, validated rule-set artifact selected by a resource binding.
pub struct ArtifactSummary {
    /// Immutable artifact identity used to bind exact historical content.
    pub id: ArtifactId,
    /// Digest of the complete normalized destination content.
    pub digest: Digest,
    /// UTC time the immutable artifact was created, distinct from later refresh checks.
    pub created_at: DateTime<Utc>,
    /// Total normalized CIDR, suffix and exact-name entries.
    pub entries: u32,
} }
meshway_serde::string_enum! {
/// Stable failure category for rule-set preparation or refresh.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProblem {
    /// DNS resolution, HTTPS transfer or its deadline failed to provide the source.
    DownloadFailed,
    /// The URL or resolved destination violates the allowed HTTPS public-source constraints.
    InvalidSource,
    /// Source content exceeds the bounded download size.
    TooLarge,
    /// The source cannot be fully converted without dropping unsupported or invalid entries.
    ConversionFailed,
    ConfigurationChanged,
    /// The process-wide download or preparation capacity is exhausted.
    BudgetExceeded,
    /// A remote rule set has no validated content bound for use yet.
    NotReady,
    /// Prepared contents fail validation against the complete current configuration and its permissions.
    ValidationFailed,
}
}
wire_object! {
/// Idempotent manual refresh of one remote rule set.
pub struct RefreshRuleSetRequest {
    /// Key retained for 24 hours per user and network; same-key retries return the current view, while reuse for another key conflicts.
    pub request_id: RequestId
} }
wire_object! { #[derive(Clone, Debug)]
/// Current rule-set declaration, selected immutable artifact and actual check status.
pub struct RuleSetView {
    /// Rule-set key in the network configuration.
    pub key: String,
    /// Current source revision owning this definition and resource view.
    pub configuration_revision: Revision,
    /// Source declaration when included in the authorized resource scope; null when the scope does not expose it.
    pub definition: Option<meshway_configuration::RuleSet>,
    /// Selected validated artifact, retained after failed refreshes; null when no artifact is bound, including inline definitions.
    pub artifact: Option<ArtifactSummary>,
    /// Last check failure or `not_ready` for missing initial remote content; null when no problem is recorded.
    pub problem: Option<ResourceProblem>,
    /// Exact execution publication whose immutable bindings select this artifact.
    pub publication: Option<PublicationId>,
    /// UTC time of the latest persisted check, successful or failed; null when no check record exists.
    pub checked_at: Option<DateTime<Utc>>,
    /// Last successful download/conversion/publication check, not artifact creation.
    pub last_success_at: Option<DateTime<Utc>>,
} }
wire_object! { #[derive(Clone, Debug)]
/// Authorized resource views for all rule sets or one requested key.
pub struct RuleSetList {
    /// Current definitions and check results, restricted to the selected key for a detail request.
    pub items: Vec<RuleSetView>
} }
