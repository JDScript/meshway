use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, Visitor},
};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::schema::{AdditionalProperties, ObjectBuilder, OneOfBuilder, Type},
};

use crate::ValidationError;

#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema,
)]
#[serde(try_from = "String")]
pub enum TlsVersion {
    #[serde(rename = "1.2")]
    Tls12,
    #[serde(rename = "1.3")]
    Tls13,
}

impl TryFrom<String> for TlsVersion {
    type Error = ValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "1.2" => Ok(Self::Tls12),
            "1.3" => Ok(Self::Tls13),
            _ => Err(ValidationError::TlsVersion),
        }
    }
}

impl TlsVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tls12 => "1.2",
            Self::Tls13 => "1.3",
        }
    }
}

/// Explicit TLS bounds; no default versions or material policy are implied.
/// Both fields are required and minVersion must not exceed maxVersion.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct TlsVersionRange {
    min_version: TlsVersion,
    max_version: TlsVersion,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionBounds {
    min_version: TlsVersion,
    max_version: TlsVersion,
}

impl TlsVersionRange {
    pub fn new(min_version: TlsVersion, max_version: TlsVersion) -> Result<Self, ValidationError> {
        if min_version > max_version {
            return Err(ValidationError::TlsVersionOrder);
        }
        Ok(Self {
            min_version,
            max_version,
        })
    }

    pub const fn min_version(self) -> TlsVersion {
        self.min_version
    }

    pub const fn max_version(self) -> TlsVersion {
        self.max_version
    }
}

impl<'de> Deserialize<'de> for TlsVersionRange {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundsVisitor;

        impl<'de> Visitor<'de> for BoundsVisitor {
            type Value = TlsVersionRange;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with explicit minVersion and maxVersion")
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let value =
                    VersionBounds::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                TlsVersionRange::new(value.min_version, value.max_version)
                    .map_err(serde::de::Error::custom)
            }
        }

        // Struct derives also accept sequence representations; the public
        // contract is an object, so reject sequences before decoding its fields.
        deserializer.deserialize_map(BoundsVisitor)
    }
}

impl PartialSchema for TlsVersionRange {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        let mut schema = OneOfBuilder::new();
        for (min, max) in [("1.2", "1.2"), ("1.2", "1.3"), ("1.3", "1.3")] {
            schema = schema.item(
                ObjectBuilder::new()
                    .schema_type(Type::Object)
                    .property(
                        "minVersion",
                        ObjectBuilder::new()
                            .schema_type(Type::String)
                            .enum_values(Some([min])),
                    )
                    .property(
                        "maxVersion",
                        ObjectBuilder::new()
                            .schema_type(Type::String)
                            .enum_values(Some([max])),
                    )
                    .required("minVersion")
                    .required("maxVersion")
                    .additional_properties(Some(AdditionalProperties::FreeForm(false))),
            );
        }
        schema.into()
    }
}

impl ToSchema for TlsVersionRange {}

/// Validate the pure WebSocket/ALPN combination before execution.
/// Empty ALPN is permitted, but does not assert runtime HTTP/1.1 support.
/// All advertised names must be exactly `http/1.1`; merely including it next
/// to `h2` is not sufficient. Order is preserved and duplicates are harmless.
pub fn validate_websocket_alpn(alpn: &[String]) -> Result<(), ValidationError> {
    if alpn.iter().all(|protocol| protocol == "http/1.1") {
        Ok(())
    } else {
        Err(ValidationError::WebSocketAlpn)
    }
}
