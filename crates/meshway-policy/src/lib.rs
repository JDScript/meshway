//! Pure policy evaluation shared by Server compilation and Node admission.
//! Callers own authentication, current credentials, leases and publication CAS.
#![forbid(unsafe_code)]

mod access;
pub mod admission;
mod authorization;
pub mod compile;
pub mod dns_route;
mod subjects;
mod validation;

pub use access::mesh_access_allows_any;
pub use authorization::{diagnostic_authorization_digest, permit_authorization_digest};
pub use compile::profile_allowed;
pub use subjects::SubjectContext;
pub use validation::*;
