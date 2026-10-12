use crate::ArtifactId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[cfg(test)]
mod tests;

/// Finite geographic coordinates with a nonnegative optional accuracy radius.
/// Private fields keep validated construction intact; the wire fields and
/// schema remain latitude, longitude and accuracyRadiusKm.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoPosition(GeoPositionFields);

crate::object! {
/// Dataset-provided approximate coordinates and optional accuracy radius.
#[derive(Clone, Debug, PartialEq)]
struct GeoPositionFields {
    /// Finite latitude in degrees, from -90 through 90.
    latitude: f64 [minimum = -90, maximum = 90],
    /// Finite longitude in degrees, from -180 through 180.
    longitude: f64 [minimum = -180, maximum = 180],
    /// Dataset provider's nonnegative estimated accuracy radius in kilometers, or null if absent.
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

crate::object! {
#[derive(Clone, Debug, PartialEq)]
/// Approximate geographic metadata returned by the Server's IP dataset.
pub struct GeoIpData {
    /// Dataset-provided ISO 3166-1 alpha-2 country or territory code, or null if absent.
    pub country: Option<String>,
    /// Dataset-provided first-level region name or identifier, or null if absent.
    pub region: Option<String>,
    /// Dataset-provided city name, or null if absent.
    pub city: Option<String>,
    /// Approximate coordinates supplied by the dataset, or null when none are available.
    pub position: Option<GeoPosition>,
}
}
crate::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// Dataset mapping from an IP address to its autonomous system, not a live routing measurement.
pub struct AsnData {
    /// Actual autonomous system number; zero is not an unknown-value placeholder.
    pub number: u32,
    /// Dataset-recorded network organization, or null if absent; this does not identify the Node owner.
    pub organization: Option<String>,
    /// Matching IP prefix returned by the dataset, or null if absent.
    pub prefix: Option<String>,
}
}

crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = LookupUnavailableReason)]
/// Reason the Server could not perform an IP metadata lookup.
pub enum UnavailableReason {
    /// No usable dataset is available for the requested metadata kind.
    DatasetUnavailable,
    /// The lookup failed instead of returning a valid dataset result.
    LookupFailed,
}
}
crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Reason public IP metadata lookup does not apply to an address.
pub enum NotApplicableReason {
    /// The address is outside the eligible public unicast address space.
    NonPublicAddress,
    /// The address belongs to this network's Mesh address space.
    MeshAddress,
}
}

/// Server IP metadata lookup result retaining dataset identity and original query time.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase", deny_unknown_fields)]
pub enum LookupResult<T> {
    /// The selected dataset contains a valid result for the address.
    Available {
        /// Immutable dataset artifact used for the lookup.
        #[serde(rename = "datasetId")]
        dataset_id: ArtifactId,
        /// UTC time of the actual lookup, retained when its result is cached.
        #[serde(rename = "lookedUpAt")]
        looked_up_at: DateTime<Utc>,
        /// Metadata returned by the selected dataset.
        data: T,
    },
    /// The lookup succeeded but the selected dataset has no entry for the address.
    NotFound {
        /// Immutable dataset artifact searched for the address.
        #[serde(rename = "datasetId")]
        dataset_id: ArtifactId,
        /// UTC time of the actual lookup, retained when its result is cached.
        #[serde(rename = "lookedUpAt")]
        looked_up_at: DateTime<Utc>,
    },
    /// No valid lookup result could be obtained.
    Unavailable {
        /// Dataset availability or lookup failure preventing a result.
        reason: UnavailableReason,
    },
    /// The address is ineligible for public IP metadata lookup.
    NotApplicable {
        /// Address classification that excludes the lookup.
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
                #[serde(deserialize_with = "crate::wire::deserialize_strict")]
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
        Ok(match Fields::deserialize(crate::wire::MapOnly(d))? {
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
crate::object! {
#[derive(Clone, Debug, PartialEq)]
/// Independent Server-derived geographic and autonomous-system lookups for one visible IP.
pub struct IpInfo {
    /// Geographic dataset result, including explicit absence or lookup failure.
    pub geoip: LookupResult<GeoIpData>,
    /// Autonomous-system dataset result, independent of geographic lookup success.
    pub asn: LookupResult<AsnData>,
}
}

crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Reason the Server names no current location address for a Node.
pub enum LocationAbsentReason {
    /// No eligible local underlay public-IP observation is available.
    NoObservation,
    /// Eligible observations have expired and cannot support a current location.
    ObservationExpired,
    /// More than one distinct address is eligible at the highest selection rank.
    AmbiguousObservation,
}
}

crate::object! {
#[derive(Clone, Debug, PartialEq, Eq)]
/// The local public underlay observation the Node's location is derived from;
/// the Server performs no geographic lookup for State.
pub struct LocationObservation {
    /// Eligible observed public underlay IP.
    pub address: String,
    /// Actual socket whose endpoint observation supplies the address.
    pub source_socket_id: String,
    /// Source socket resource generation associated with the observation.
    pub source_generation: crate::Sequence,
    /// Local observation source; configured declarations alone cannot establish a Node location.
    pub endpoint_source: LocationSource,
    /// Original UTC time of the endpoint observation.
    pub observed_at: DateTime<Utc>,
    /// Original UTC endpoint deadline limiting this location's current validity.
    pub expires_at: DateTime<Utc>,
}
}

crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Address class of a visible endpoint IP.
pub enum AddressScope {
    /// Globally routable unicast address.
    Public,
    /// Private, shared, link-local, loopback, documentation or other reserved address.
    NonPublic,
    /// A Mesh address of this network.
    Mesh,
}
}
crate::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
/// Eligible local endpoint observation used to derive approximate Node location.
pub enum LocationSource {
    /// Public underlay address observed on a usable local interface.
    Interface,
    /// Public mapping observed by STUN on the actual UDP socket.
    Stun,
}
}
