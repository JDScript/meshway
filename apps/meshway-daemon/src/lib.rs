//! Node application lifecycle, shared by the executable and embedded hosts.
#![forbid(unsafe_code)]
#![cfg(unix)]
mod ipc;
mod runtime;
mod settings;
mod status;
mod store;
mod worker;
pub use runtime::run;
pub use settings::Settings;
