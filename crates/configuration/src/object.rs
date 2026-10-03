use std::{collections::BTreeMap, marker::PhantomData};
pub(crate) struct MapOnly<D>(pub(crate) D);
impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for MapOnly<D> {
    type Error = D::Error;
    fn deserialize_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        struct OnlyMap<V>(V);
        impl<'de, V: serde::de::Visitor<'de>> serde::de::Visitor<'de> for OnlyMap<V> {
            type Value = V::Value;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                map: M,
            ) -> Result<Self::Value, M::Error> {
                self.0.visit_map(map)
            }
        }
        self.0.deserialize_map(OnlyMap(visitor))
    }
    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}
macro_rules! object {
    ($(#[$attr:meta])* pub struct $name:ident {
        $($(#[$field_attr:meta])* pub $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)?
    }) => {
        $(#[$attr])*
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name { $($(#[$field_attr])* $(#[schema($($schema)*)])? pub $field: $ty),* }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Fields { $($(#[$field_attr])* $field: $ty),* }
                let Fields { $($field),* } = Fields::deserialize($crate::object::MapOnly(d))?;
                Ok(Self { $($field),* })
            }
        }
    };
}
pub(crate) use object;

macro_rules! tagged {
    (pub enum $name:ident { $($variant:ident $({ $($(#[$attr:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? })?),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
        #[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
        pub enum $name { $($variant $({ $($(#[$attr])* $field: $ty),* })?),* }
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self,D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
                enum Fields { $($variant { $($($(#[$attr])* $field: $ty),*)? }),* }
                Ok(match Fields::deserialize($crate::object::MapOnly(d))? { $(Fields::$variant { $($($field),*)? } => Self::$variant $({ $($field),* })?),* })
            }
        }
        impl utoipa::PartialSchema for $name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                use utoipa::openapi::{RefOr, schema::{Schema, ObjectBuilder, OneOfBuilder, Type}};
                let mut union = OneOfBuilder::new();
                $(
                    $crate::object::object! { pub struct $variant { $($($(#[$attr])* pub $field: $ty $([$($schema)*])?),*)? } }
                    let RefOr::T(Schema::Object(mut object)) = <$variant as utoipa::PartialSchema>::schema() else { unreachable!("object macro emits an object") };
                    let name = stringify!($variant);
                    let tag = format!("{}{}", name[..1].to_ascii_lowercase(), &name[1..]);
                    object.properties.insert("type".into(), ObjectBuilder::new().schema_type(Type::String).enum_values(Some([tag])).into());
                    object.required.push("type".into());
                    union = union.item(object);
                )*
                union.into()
            }
        }
        impl utoipa::ToSchema for $name {
            fn schemas(schemas: &mut Vec<(String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>)>) {
                $(
                    $crate::object::object! { pub struct $variant { $($($(#[$attr])* pub $field: $ty $([$($schema)*])?),*)? } }
                    <$variant as utoipa::ToSchema>::schemas(schemas);
                )*
            }
        }
    }
}
pub(crate) use tagged;

pub(crate) fn unique_map<'de, D, K, V>(d: D) -> Result<BTreeMap<K, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    K: serde::Deserialize<'de> + Ord,
    V: serde::Deserialize<'de>,
{
    struct Entries<K, V>(PhantomData<(K, V)>);
    impl<'de, K: serde::Deserialize<'de> + Ord, V: serde::Deserialize<'de>> serde::de::Visitor<'de>
        for Entries<K, V>
    {
        type Value = BTreeMap<K, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("an object with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut m: M,
        ) -> Result<Self::Value, M::Error> {
            let mut values = BTreeMap::new();
            while let Some((k, v)) = m.next_entry()? {
                if values.insert(k, v).is_some() {
                    return Err(serde::de::Error::custom("duplicate map key"));
                }
            }
            Ok(values)
        }
    }
    d.deserialize_map(Entries(PhantomData))
}
