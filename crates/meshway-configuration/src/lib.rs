//! Network configuration values and deterministic configuration processing.
//!
//! Persistence, material generation, authentication, and machine resources are
//! owned by the Controller and Node applications, not this library.

#![forbid(unsafe_code)]

mod digest;
pub use digest::{SerializationError, digest};
mod dns_name;
pub use dns_name::{DnsNameError, MAX_DNS_NAME_INPUT_BYTES, node_dns_name, normalize_dns_name};
mod document;
mod editing;
mod editing_secrets;
pub mod evaluation;
pub mod execution;
pub use editing_secrets::*;
mod fields;
mod object;
mod source;
mod types;
mod validation;
mod view;
pub use source::{edit_source, rewrite_source};
mod identity;
mod internet;
pub use document::*;
pub use editing::*;
pub use fields::*;
pub use identity::{rebind_node_user_references, rebind_policy_user_references};
pub use internet::{AddressSet, internet_ranges};
pub use meshway_transport_spec as transport;
pub use meshway_transport_spec::{OuterSocket, TransportProtocol};
pub use types::*;
pub use validation::*;
pub use view::*;
