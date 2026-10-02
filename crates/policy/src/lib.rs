//! Pure four-layer network policy compilation. No I/O or packet execution.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use utoipa::ToSchema;
mod compiled;
mod engine;
mod graph;
mod index;
mod internet;
mod mesh_lints;
mod model;
mod path_search;
mod profile_plan;
mod profile_projection;
mod profile_validation;
mod profiles;
mod rule_sets;
mod validation;
pub use compiled::*;
pub use engine::compile;
pub use index::{DestinationIndex, FlattenedPrefix};
pub use model::*;
pub use profiles::*;
pub use rule_sets::*;
pub use validation::validate;
pub(crate) use model::{allowed, default_rules, matches, specificity};
#[cfg(test)]
mod tests;

pub fn valid_tag_selector(value: &str) -> bool {
    let Some(name) = value.strip_prefix("tag:") else { return false; };
    !name.is_empty() && name.len() <= 48 && name.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index != 0 && (byte == b'_' || byte == b'-'))
    })
}
