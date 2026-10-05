use crate::ArtifactId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[cfg(test)]
mod tests;

meshway_serde::object! {
/// Canonical immutable rule-set content, shared with configuration compilation.
/// `domain` entries match one exact DNS name; `domain_suffix` entries match the
/// name and every child label. Artifacts stored before `domain` existed have
/// none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSetData {
    pub ip_cidr: Vec<String>,
    pub domain_suffix: Vec<String>,
    #[serde(default)]
    pub domain: Vec<String>,
}
}

/// Finite geographic coordinates with a nonnegative optional accuracy radius.
/// Private fields keep validated construction intact; the wire fields and
/// schema remain latitude, longitude and accuracyRadiusKm.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoPosition(GeoPositionFields);

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq)]
struct GeoPositionFields {
    latitude: f64 [minimum = -90, maximum = 90],
    longitude: f64 [minimum = -180, maximum = 180],
    accuracy_radius_km: Option<f64> [minimum = 0],
}
}
impl utoipa::PartialSchema for GeoPosition {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <GeoPositionFields as utoipa::PartialSchema>::schema()
    }
}
impl ToSchema for GeoPosition {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected finite geographic coordinates and a nonnegative accuracy radius")]
pub struct InvalidGeoPosition;

impl GeoPosition {
    pub fn new(
        latitude: f64,
        longitude: f64,
        accuracy_radius_km: Option<f64>,
    ) -> Result<Self, InvalidGeoPosition> {
        let value = Self(GeoPositionFields {
            latitude,
            longitude,
            accuracy_radius_km,
        });
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), InvalidGeoPosition> {
        if !(-90.0..=90.0).contains(&self.0.latitude)
            || !(-180.0..=180.0).contains(&self.0.longitude)
            || self
                .0
                .accuracy_radius_km
                .is_some_and(|radius| !radius.is_finite() || radius < 0.0)
        {
            return Err(InvalidGeoPosition);
        }
        Ok(())
    }

    pub fn latitude(&self) -> f64 {
        self.0.latitude
    }
    pub fn longitude(&self) -> f64 {
        self.0.longitude
    }
    pub fn accuracy_radius_km(&self) -> Option<f64> {
        self.0.accuracy_radius_km
    }
}
impl<'de> Deserialize<'de> for GeoPosition {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Self(GeoPositionFields::deserialize(d)?);
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}
impl Serialize for GeoPosition {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.validate().map_err(serde::ser::Error::custom)?;
        self.0.serialize(s)
    }
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq)]
pub struct GeoIpData {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub position: Option<GeoPosition>,
}
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AsnData {
    pub number: u32,
    pub organization: Option<String>,
    pub prefix: Option<String>,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = LookupUnavailableReason)]
pub enum UnavailableReason {
    DatasetUnavailable,
    LookupFailed,
}
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum NotApplicableReason {
    NonPublicAddress,
    MeshAddress,
}
}

#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
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
impl<'de, T: Deserialize<'de>> Deserialize<'de> for LookupResult<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
        #[serde(bound(deserialize = "T: Deserialize<'de>"))]
        enum Fields<T> {
            Available {
                #[serde(rename = "datasetId")]
                dataset_id: ArtifactId,
                #[serde(rename = "lookedUpAt")]
                looked_up_at: DateTime<Utc>,
                #[serde(deserialize_with = "meshway_serde::deserialize_strict")]
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
        Ok(match Fields::deserialize(meshway_serde::MapOnly(d))? {
            Fields::Available {
                dataset_id,
                looked_up_at,
                data,
            } => Self::Available {
                dataset_id,
                looked_up_at,
                data,
            },
            Fields::NotFound {
                dataset_id,
                looked_up_at,
            } => Self::NotFound {
                dataset_id,
                looked_up_at,
            },
            Fields::Unavailable { reason } => Self::Unavailable { reason },
            Fields::NotApplicable { reason } => Self::NotApplicable { reason },
        })
    }
}
meshway_serde::object! {
#[derive(Clone, Debug, PartialEq)]
pub struct IpInfo {
    pub geoip: LookupResult<GeoIpData>,
    pub asn: LookupResult<AsnData>,
}
}

meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LocationAbsentReason {
    NoObservation,
    ObservationExpired,
    DatasetUnavailable,
    NotInDataset,
    LookupFailed,
    AmbiguousObservation,
}
}

meshway_serde::object! {
#[derive(Clone, Debug, PartialEq)]
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
}
meshway_serde::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LocationSource {
    Interface,
    Stun,
}
}
