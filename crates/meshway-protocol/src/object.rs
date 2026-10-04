pub(crate) use meshway_serde::{MapOnly, object as wire_object, tagged as wire_tagged};

// A field-level deserializer makes missing fields fail in Serde while keeping
// explicit null valid, matching `required=true` nullable response schemas.
pub(crate) fn required_nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    serde::Deserialize::deserialize(deserializer)
}
