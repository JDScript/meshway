//! Common JSON object rules for configuration, transport and HTTP contracts.
//! Domain values and HTTP behavior belong to their respective crates. The
//! declaration macros `object!`, `tagged!` and `string_enum!` are exported at
//! the crate root.

mod declarations;
mod strict;

pub use strict::{MapOnly, deserialize_strict, unique_map};

/// A present field may contain null, but omission is an error. Use with
/// `serde(deserialize_with)` and a required nullable schema annotation.
pub fn required_nullable<'de, D: serde::Deserializer<'de>, T: serde::Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    serde::Deserialize::deserialize(deserializer)
}
