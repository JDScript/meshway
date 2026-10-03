use crate::{Digest, KeyId, PublicationId, Revision, Version};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Control signing and dataplane encryption are deliberately different purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeyPurpose {
    Control,
    Dataplane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyAlgorithm {
    Ed25519,
    X25519,
}

/// Canonical hex key bytes. Algorithms are checked against their purpose at binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodePublicKey {
    pub algorithm: KeyAlgorithm,
    pub bytes: Digest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundNodeKey {
    pub id: KeyId,
    pub generation: Version,
    pub public_key: NodePublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigurationRef {
    pub revision: Revision,
    pub execution_publication: PublicationId,
    pub digest: Digest,
}

// Force wire objects through map access: serde's default struct visitor also
// accepts positional sequences, which are outside the HTTP schema contract.
struct MapOnly<D>(D);
impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for MapOnly<D> {
    type Error = D::Error;
    fn deserialize_any<V: serde::de::Visitor<'de>>(self, v: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(v)
    }
    serde::forward_to_deserialize_any! {bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map struct enum identifier ignored_any}
}
macro_rules! decode_object {
    ($name:ident {$($field:ident:$ty:ty),*})=>{
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D:serde::Deserializer<'de>>(d:D)->Result<Self,D::Error>{
                #[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]
                struct Fields {$($field:$ty),*}
                let Fields{$($field),*}=Fields::deserialize(MapOnly(d))?;Ok(Self{$($field),*})
            }
        }
    }
}
decode_object!(NodePublicKey {
    algorithm: KeyAlgorithm,
    bytes: Digest
});
decode_object!(BoundNodeKey {
    id: KeyId,
    generation: Version,
    public_key: NodePublicKey
});
decode_object!(ConfigurationRef {
    revision: Revision,
    execution_publication: PublicationId,
    digest: Digest
});
