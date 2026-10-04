//! Common JSON object rules for configuration, transport and HTTP contracts.
//! Domain values and HTTP behavior belong to their respective crates.
#![forbid(unsafe_code)]

mod declarations;
mod strict;

pub use strict::{MapOnly, unique_map};
