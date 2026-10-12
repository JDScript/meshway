use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;
use zeroize::Zeroize;

/// Serialization is secret-bearing and requires protected storage.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = String)]
pub struct SecretString(String);
impl SecretString {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A listener credential or private key: its value, or the `{"retained": true}`
/// marker a redacted read shows for a saved value. Submitting the marker keeps
/// the saved value of the same listener; it never stands for a new value.
/// Prepared listeners and every execution input hold values only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenerSecret {
    Value(SecretString),
    Retained,
}
impl ListenerSecret {
    pub fn value(&self) -> Option<&SecretString> {
        match self {
            Self::Value(value) => Some(value),
            Self::Retained => None,
        }
    }
    pub fn is_retained(&self) -> bool {
        matches!(self, Self::Retained)
    }
}
impl From<SecretString> for ListenerSecret {
    fn from(value: SecretString) -> Self {
        Self::Value(value)
    }
}
impl Serialize for ListenerSecret {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            Self::Value(value) => value.serialize(s),
            Self::Retained => {
                let mut map = s.serialize_map(Some(1))?;
                map.serialize_entry("retained", &true)?;
                map.end()
            }
        }
    }
}
impl<'de> Deserialize<'de> for ListenerSecret {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = ListenerSecret;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#"a string or {"retained": true}"#)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(ListenerSecret::Value(SecretString::new(v.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(ListenerSecret::Value(SecretString::new(v)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                use serde::de::Error;
                let key: Option<String> = map.next_key()?;
                if key.as_deref() != Some("retained") || !map.next_value::<bool>()? {
                    return Err(A::Error::custom("invalid retained secret marker"));
                }
                if map.next_key::<String>()?.is_some() {
                    return Err(A::Error::custom("invalid retained secret marker"));
                }
                Ok(ListenerSecret::Retained)
            }
        }
        d.deserialize_any(Visitor)
    }
}
impl utoipa::PartialSchema for ListenerSecret {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{AdditionalProperties, ObjectBuilder, OneOfBuilder, Type};
        OneOfBuilder::new()
            .description(Some(
                "The secret value, or `{\"retained\": true}`: a saved value withheld by a read, \
                 which keeps the same listener's saved value when submitted.",
            ))
            .item(ObjectBuilder::new().schema_type(Type::String))
            .item(
                ObjectBuilder::new()
                    .schema_type(Type::Object)
                    .property(
                        "retained",
                        ObjectBuilder::new()
                            .schema_type(Type::Boolean)
                            .enum_values(Some([true])),
                    )
                    .required("retained")
                    .additional_properties(Some(AdditionalProperties::FreeForm(false))),
            )
            .into()
    }
}
impl ToSchema for ListenerSecret {}
