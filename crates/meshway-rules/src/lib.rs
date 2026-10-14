//! Meshway rule sets: the `MWRS` v1 binary artifact and everything that reads
//! or produces it.
//!
//! - [`Builder`] turns normalized [`Entry`] values into a deterministic
//!   artifact: equal entry sets give equal bytes and therefore equal
//!   [`Digest`]s.
//! - [`RuleSet`] is the verified, zero-deserialization loader and matcher
//!   over a heap buffer ([`RuleSet::from_vec`]). Any byte
//!   string is either rejected or yields a set whose queries cannot panic.
//! - [`inline`] converts configuration lists; `convert` (feature `convert`)
//!   converts sing-box source, mihomo and classical payloads, always with a
//!   per-entry [`Report`].
//! - `catalog` (feature `catalog`) is the signed provider `index.json`.
//!
//! Layout and semantics are defined in `docs/v2/rule-sets-design.md`.

mod build;
mod digest;
mod domains;
mod format;
pub mod inline;
mod name;
pub mod ranges;
mod report;
mod set;

#[cfg(feature = "catalog")]
pub mod catalog;
#[cfg(feature = "convert")]
pub mod convert;

pub use build::{Artifact, BuildError, Builder, Limit};
pub use digest::{Digest, InvalidDigest};
pub use format::{
    Counts, MAJOR, MAX_ARTIFACT_BYTES, MAX_KEYWORDS, MAX_NAMES, MAX_RANGES, MEDIA_TYPE, MINOR,
};
pub use name::{Cidr, Entry, InvalidCidr, InvalidKeyword, InvalidName, Keyword, Name};
pub use report::{Conversion, ConvertError, Converted, Reason, Report, Skipped, Summary};
pub use set::{DomainEntry, DomainHit, DomainKind, LoadError, RuleSet, SectionInfo};
