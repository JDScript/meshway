use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, PublicationId, RequestId, Revision};

pub use meshway_model::resources::RuleSetData;
wire_object! { #[derive(Clone, Debug)] pub struct PreparedRuleSet {
    pub digest: Digest, pub entries: u32,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ArtifactSummary {
    pub id: ArtifactId, pub digest: Digest, pub created_at: DateTime<Utc>, pub entries: u32,
} }
meshway_serde::string_enum! {
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProblem {
    DownloadFailed,
    InvalidSource,
    TooLarge,
    ConversionFailed,
    ConfigurationChanged,
    BudgetExceeded,
    NotReady,
    ValidationFailed,
}
}
wire_object! { pub struct RefreshRuleSetRequest { pub request_id: RequestId } }
wire_object! { #[derive(Clone, Debug)] pub struct RuleSetView {
    pub key: String, pub configuration_revision: Revision,
    pub definition: Option<meshway_configuration::RuleSet>,
    pub artifact: Option<ArtifactSummary>, pub problem: Option<ResourceProblem>,
    /// Exact execution publication whose immutable bindings select this artifact.
    pub publication: Option<PublicationId>,
    pub checked_at: Option<DateTime<Utc>>,
    /// Last successful download/conversion/publication check, not artifact creation.
    pub last_success_at: Option<DateTime<Utc>>,
} }
wire_object! { #[derive(Clone, Debug)] pub struct RuleSetList { pub items: Vec<RuleSetView> } }
