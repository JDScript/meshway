//! Node application lifecycle, shared by the executable and embedded hosts.
#![forbid(unsafe_code)]
#![cfg(unix)]
mod ipc;
mod runtime;
mod service;
mod settings;
pub use runtime::run;
pub use settings::Settings;
