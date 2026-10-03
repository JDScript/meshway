//! Network configuration values and deterministic configuration processing.
//!
//! Persistence, material generation, authentication, and machine resources are
//! owned by the Controller and Node applications, not this library.

#![forbid(unsafe_code)]

mod document;
mod editing;
mod fields;
mod object;
mod source;
mod types;
mod validation;
mod view;
pub use source::{edit_source, rewrite_source};
mod identity;
pub use document::*;
pub use editing::*;
pub use fields::*;
pub use identity::{rebind_user_references, rename_user_references};
pub use meshway_transport_spec::{OuterSocket, TransportProtocol};
pub use types::*;
pub use validation::*;
pub use view::*;
