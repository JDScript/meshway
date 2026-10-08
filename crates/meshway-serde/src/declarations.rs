/// A string-only enum. The declaration supplies its Serialize and schema
/// derives. One variant/rename list also declares the private decoder; no raw
/// remote-derive deserializer is exposed on the public type.
#[macro_export]
macro_rules! string_enum {
    ($(#[$attribute:meta])* pub enum $name:ident {
        $($(#[$variant_attribute:meta])* $variant:ident),* $(,)?
    }) => {
        $(#[$attribute])*
        pub enum $name { $($(#[$variant_attribute])* $variant),* }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                $(#[$attribute])*
                #[derive(serde::Deserialize)]
                enum Fields { $($(#[$variant_attribute])* $variant),* }
                let value = <String as serde::Deserialize>::deserialize(d)?;
                Ok(match <Fields as serde::Deserialize>::deserialize(
                    serde::de::value::StringDeserializer::<D::Error>::new(value),
                )? {
                    $(Fields::$variant => Self::$variant),*
                })
            }
        }
    };
}

/// One field declaration drives Serde and its closed OpenAPI object schema.
/// Consumers use their existing `serde` and `utoipa` dependencies. Optional
/// schema attributes follow a field type in brackets, separate from Serde attrs.
#[macro_export]
macro_rules! object {
    ($(#[$attribute:meta])* $struct_visibility:vis struct $name:ident {
        $($(#[$field_attribute:meta])* $visibility:vis $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)?
    }) => {
        $(#[$attribute])*
        #[derive(serde::Serialize, utoipa::ToSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        $struct_visibility struct $name {
            $($(#[$field_attribute])* $(#[schema($($schema)*)])? $visibility $field: $ty),*
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                #[derive(serde::Deserialize)]
                #[serde(rename_all = "camelCase", deny_unknown_fields)]
                struct Fields { $($(#[$field_attribute])* $field: $ty),* }
                let Fields { $($field),* } = Fields::deserialize($crate::MapOnly(d))?;
                Ok(Self { $($field),* })
            }
        }
    };
}

/// Closed internally tagged objects, including unit and nullable-field branches.
/// Each schema branch is built from its payload object rather than intersecting
/// a closed payload with an extra discriminator via an unsatisfiable `allOf`.
#[macro_export]
macro_rules! tagged {
    (@name $variant:ident) => {{
        let name = stringify!($variant);
        format!("{}{}", name[..1].to_ascii_lowercase(), &name[1..])
    }};
    (@name $variant:ident, $wire:literal) => { String::from($wire) };
    ($(#[$attribute:meta])* pub enum $name:ident { $($variants:tt)* }) => {
        $crate::tagged!(tag="type"; $(#[$attribute])* pub enum $name { $($variants)* });
    };
    (tag=$tag:literal; $(#[$attribute:meta])* pub enum $name:ident {
        $($(#[$variant_attribute:meta])* $variant:ident $(= $wire:literal)? $({ $($(#[$field_attribute:meta])* $field:ident: $ty:ty $([$($schema:tt)*])?),* $(,)? })?),* $(,)?
    }) => {
        $(#[$attribute])*
        #[derive(serde::Serialize)]
        #[serde(tag = $tag, rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
        pub enum $name { $($(#[$variant_attribute])* $(#[serde(rename = $wire)])? $variant $({ $($(#[$field_attribute])* $field: $ty),* })?),* }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                // Serde buffers an internally tagged enum before decoding its
                // payload. Decode each buffered payload through the object
                // boundary again so nested plain structs cannot become arrays.
                $(
                    $crate::object! { pub struct $variant {
                        $($($(#[$field_attribute])* pub $field: $ty $([$($schema)*])?),*)?
                    } }
                )*
                #[derive(serde::Deserialize)]
                #[serde(tag = $tag, rename_all = "camelCase", deny_unknown_fields)]
                enum Fields { $($(#[serde(rename = $wire)])? $variant($variant)),* }
                Ok(match Fields::deserialize($crate::MapOnly(d))? {
                    $(Fields::$variant($variant { $($($field),*)? }) => Self::$variant $({ $($field),* })?),*
                })
            }
        }

        impl utoipa::PartialSchema for $name {
            fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
                use utoipa::openapi::{RefOr, schema::{Schema, ObjectBuilder, OneOfBuilder, Type}};
                // Doc comments on the enum and its variants become the union's
                // and each branch's description; the derive sees them only on
                // these stand-in declarations.
                $crate::object! { $(#[$attribute])* pub struct $name {} }
                let description = match <$name as utoipa::PartialSchema>::schema() {
                    RefOr::T(Schema::Object(object)) => object.description,
                    _ => None,
                };
                let mut union = OneOfBuilder::new().description(description);
                $(
                    $crate::object! { $(#[$variant_attribute])* pub struct $variant {
                        $($($(#[$field_attribute])* pub $field: $ty $([$($schema)*])?),*)?
                    } }
                    let RefOr::T(Schema::Object(mut object)) = <$variant as utoipa::PartialSchema>::schema() else {
                        unreachable!("object declaration emits an object schema")
                    };
                    let tag = $crate::tagged!(@name $variant $(, $wire)?);
                    object.properties.insert($tag.into(), ObjectBuilder::new()
                        .schema_type(Type::String).enum_values(Some([tag])).into());
                    object.required.push($tag.into());
                    union = union.item(object);
                )*
                union.into()
            }
        }

        impl utoipa::ToSchema for $name {
            fn schemas(schemas: &mut Vec<(String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>)>) {
                $(
                    $crate::object! { pub struct $variant {
                        $($($(#[$field_attribute])* pub $field: $ty $([$($schema)*])?),*)?
                    } }
                    <$variant as utoipa::ToSchema>::schemas(schemas);
                )*
            }
        }
    };
}
