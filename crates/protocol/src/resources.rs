use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{ArtifactId, Digest, Revision};

pub use meshway_model::resources::RuleSetData;
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)] pub struct ArtifactSummary {
    pub id: ArtifactId, pub digest: Digest, pub created_at: DateTime<Utc>, pub entries: u32,
} }
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProblem {
    DownloadFailed,
    InvalidSource,
    TooLarge,
    ConversionFailed,
    ConfigurationChanged,
    BudgetExceeded,
}
wire_object! { #[derive(Clone, Debug)] pub struct RuleSetView {
    pub key: String, pub configuration_revision: Revision,
    pub definition: Option<meshway_configuration::RuleSet>,
    pub artifact: Option<ArtifactSummary>, pub problem: Option<ResourceProblem>,
    pub checked_at: Option<DateTime<Utc>>,
} }
wire_object! { #[derive(Clone, Debug)] pub struct RuleSetList { pub items: Vec<RuleSetView> } }
