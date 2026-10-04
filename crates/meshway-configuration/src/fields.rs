use serde::{Deserialize, Serialize};
use utoipa::{PartialSchema, ToSchema};

/// Editing intent: omission is different from an explicit value (including
/// null when T itself is nullable). Never use this as an execution value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Input<T> {
    #[default]
    Omitted,
    Value(T),
}
impl<T> Input<T> {
    pub fn is_omitted(&self) -> bool {
        matches!(self, Self::Omitted)
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Input<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::deserialize(d).map(Self::Value)
    }
}
impl<T: Serialize> Serialize for Input<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Value(v) => v.serialize(s),
            Self::Omitted => Err(serde::ser::Error::custom(
                "omitted input field must be skipped",
            )),
        }
    }
}
impl<T: PartialSchema> PartialSchema for Input<T> {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        T::schema()
    }
}
impl<T: ToSchema> ToSchema for Input<T> {
    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        T::schemas(schemas);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum AccessDefault {
    #[default]
    Deny,
}
impl TryFrom<String> for AccessDefault {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        if v == "deny" {
            Ok(Self::Deny)
        } else {
            Err("access default must be deny")
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TopologyDefault {
    #[default]
    Allow,
    Deny,
}
impl TryFrom<String> for TopologyDefault {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "allow" => Ok(Self::Allow),
            "deny" => Ok(Self::Deny),
            _ => Err("invalid topology default"),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TopologyAction {
    Allow,
    Block,
}
impl TryFrom<String> for TopologyAction {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "allow" => Ok(Self::Allow),
            "block" => Ok(Self::Block),
            _ => Err("invalid topology action"),
        }
    }
}
