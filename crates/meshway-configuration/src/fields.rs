use serde::{Deserialize, Serialize};
use utoipa::{PartialSchema, ToSchema};

/// Editing intent: omission is different from an explicit value (including
/// null when T itself is nullable). Never use this as an execution value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Input<T> {
    /// No field was supplied; the containing input determines whether to retain or inherit its value.
    #[default]
    Omitted,
    /// An explicit replacement, including null only when the field's value permits it.
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

/// Behavior for Mesh business traffic without an explicit access grant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum AccessDefault {
    /// Deny unmatched traffic; the only supported default.
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
/// Adjacency permission when neither a block nor an allow topology rule matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TopologyDefault {
    /// Permit adjacency without an edge-specific transport restriction; the default value.
    #[default]
    Allow,
    /// Deny adjacency unless an explicit allow rule matches.
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
/// Permission applied to a matching undirected node pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum TopologyAction {
    /// Permit adjacency if no block matches, using the first matching allow rule's transport restriction.
    Allow,
    /// Reject the entire adjacency regardless of any matching allow rule.
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
