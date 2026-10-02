//! Generated Controller consumers. State validates and persists their results;
//! daemon owns scheduling/application. All consumers use Agent's bound client.
mod configuration;
#[cfg(all(target_os = "linux", feature = "managed-connections"))]
mod protection;
#[cfg(feature = "managed-connections")]
pub(crate) mod transport;
#[cfg(all(target_os = "linux", feature = "managed-connections"))]
pub(crate) use protection::{Bootstrap, Control};
