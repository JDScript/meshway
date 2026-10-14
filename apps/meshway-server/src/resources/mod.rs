//! Bounded rule-set preparation (download, catalog verification, conversion),
//! global content-addressed artifacts and immutable IP datasets. GeoIP
//! datasets are supplied by deployment, never by network administrators or
//! Node reports.
pub(crate) mod artifacts;
pub(crate) mod bindings;
pub(crate) mod browse;
pub(crate) mod catalogs;
pub mod datasets;
pub mod download;
pub(crate) mod downloads;
mod geoip;
pub mod provider;
pub(crate) mod rulesets;
pub(crate) mod scheduler;
pub mod service;
pub use geoip::GeoIp;
pub(crate) use geoip::is_public;

use meshway_model::Digest;
use sha2::{Digest as _, Sha256};

/// Remote rule sets one configuration may need to download in a single edit.
pub const MAX_REMOTE_RULE_SETS: usize = 32;

pub fn content_digest(bytes: &[u8]) -> Digest {
    Digest::from_bytes(Sha256::digest(bytes).into())
}
