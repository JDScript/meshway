//! Pure policy evaluation shared by Server compilation and Node admission.
//! Callers own authentication, current credentials, leases and publication CAS.
#![forbid(unsafe_code)]

mod access;
pub mod admission;
pub mod compile;
pub mod dns_route;
mod internet;
pub use internet::{exit_exclusions, internet_ranges, is_exit_destination, is_internet_address};
mod subjects;
mod validation;

pub use access::mesh_access_allows_any;
pub use compile::profile_allowed;
pub use subjects::SubjectContext;
pub use validation::*;
