//! Serde structs also accept sequence input by default. Wire object schemas do
//! not. The macro retains one field declaration for Serde and OpenAPI while
//! routing public Deserialize through map-only deserialization. Optional schema
//! attributes follow the field type in brackets; they are not Serde attributes.

pub(crate) struct MapOnly<D>(pub(crate) D);

impl<'de, D: serde::Deserializer<'de>> serde::Deserializer<'de> for MapOnly<D> {
    type Error = D::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(visitor)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map struct enum identifier ignored_any
    }
}

macro_rules! wire_object {
    ($(#[$attribute:meta])* pub struct $name:ident {
        $($(#[$field_attribute:meta])* $visibility:vis $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)?
    }) => {
        $(#[$attribute])*
        #[derive(serde::Serialize, utoipa::ToSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            $($(#[$field_attribute])* $(#[schema($($schema)*)])? $visibility $field: $ty),*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Fields { $($(#[$field_attribute])* $field: $ty),* }
                let Fields { $($field),* } = Fields::deserialize($crate::object::MapOnly(deserializer))?;
                Ok(Self { $($field),* })
            }
        }
    };
}

pub(crate) use wire_object;
