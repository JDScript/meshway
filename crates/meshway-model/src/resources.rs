use crate::ArtifactId;

/// Canonical immutable rule-set content, shared with configuration compilation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleSetData {
    pub ip_cidr: Vec<String>,
    pub domain_suffix: Vec<String>,
}
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeoPosition {
    #[schema(minimum = -90, maximum = 90)]
    pub latitude: f64,
    #[schema(minimum = -180, maximum = 180)]
    pub longitude: f64,
    #[schema(minimum = 0)]
    pub accuracy_radius_km: Option<f64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeoIpData {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub position: Option<GeoPosition>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AsnData {
    pub number: u32,
    pub organization: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = LookupUnavailableReason)]
pub enum UnavailableReason {
    DatasetUnavailable,
    LookupFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum NotApplicableReason {
    NonPublicAddress,
    MeshAddress,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum LookupResult<T> {
    Available {
        #[serde(rename = "datasetId")]
        dataset_id: ArtifactId,
        #[serde(rename = "lookedUpAt")]
        looked_up_at: DateTime<Utc>,
        data: T,
    },
    NotFound {
        #[serde(rename = "datasetId")]
        dataset_id: ArtifactId,
        #[serde(rename = "lookedUpAt")]
        looked_up_at: DateTime<Utc>,
    },
    Unavailable {
        reason: UnavailableReason,
    },
    NotApplicable {
        reason: NotApplicableReason,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpInfo {
    pub geoip: LookupResult<GeoIpData>,
    pub asn: LookupResult<AsnData>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LocationAbsentReason {
    NoObservation,
    ObservationExpired,
    DatasetUnavailable,
    NotInDataset,
    LookupFailed,
    AmbiguousObservation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GeoObservation {
    pub address: String,
    pub source_socket_id: String,
    pub source_generation: crate::Sequence,
    pub dataset_id: ArtifactId,
    pub data: GeoIpData,
    pub endpoint_source: LocationSource,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LocationSource {
    Interface,
    Stun,
}
