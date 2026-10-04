//! Shared public network/member fragments, independent of HTTP envelopes.
use crate::{MembershipId, MembershipRole, NetworkId, Revision, UserId, Version};
use chrono::{DateTime, Utc};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(remote = "Self", rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkActions {
    pub rename: bool,
    pub edit_configuration: bool,
    pub manage_members: bool,
    pub join_link_roles: Vec<MembershipRole>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(remote = "Self", rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkSummary {
    pub id: NetworkId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub current_revision: Revision,
    pub membership_id: MembershipId,
    pub role: MembershipRole,
    pub actions: NetworkActions,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(remote = "Self", rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberActions {
    pub change_role: bool,
    pub assignable_roles: Vec<MembershipRole>,
    pub remove: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(remote = "Self", rename_all = "camelCase", deny_unknown_fields)]
pub struct MemberSummary {
    pub membership_id: MembershipId,
    pub user_id: UserId,
    pub email: String,
    pub role: MembershipRole,
    pub version: Version,
    pub joined_at: DateTime<Utc>,
    pub actions: MemberActions,
}

// Remote derives keep one field declaration while allowing map-only decoding.
// These fragments must retain the same object boundary as their envelopes.
trait Fields: Sized {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error>;
}
fn object<'de, T: Fields, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    struct Map<T>(std::marker::PhantomData<T>);
    impl<'de, T: Fields> Visitor<'de> for Map<T> {
        type Value = T;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a JSON object")
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
            T::read(MapAccessDeserializer::new(map))
        }
    }
    d.deserialize_map(Map(std::marker::PhantomData))
}
impl Fields for NetworkActions {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
impl<'de> Deserialize<'de> for NetworkActions {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        object(d)
    }
}
impl Serialize for NetworkActions {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, s)
    }
}
impl Fields for NetworkSummary {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
impl<'de> Deserialize<'de> for NetworkSummary {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        object(d)
    }
}
impl Serialize for NetworkSummary {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, s)
    }
}
impl Fields for MemberActions {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
impl<'de> Deserialize<'de> for MemberActions {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        object(d)
    }
}
impl Serialize for MemberActions {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, s)
    }
}
impl Fields for MemberSummary {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::deserialize(d)
    }
}
impl<'de> Deserialize<'de> for MemberSummary {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        object(d)
    }
}
impl Serialize for MemberSummary {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Self::serialize(self, s)
    }
}
