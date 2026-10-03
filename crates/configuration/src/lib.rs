//! Network configuration values and deterministic configuration processing.
//!
//! Persistence, material generation, authentication, and machine resources are
//! owned by the Controller and Node applications, not this library.

#![forbid(unsafe_code)]

mod document;
mod fields;
mod object;
mod types;
mod validation;
pub use document::*;
pub use fields::*;
pub use types::*;
pub use validation::*;
