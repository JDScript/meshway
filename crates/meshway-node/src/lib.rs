//! The Node's single runtime, shared by authenticated local hosts.
#![forbid(unsafe_code)]

pub mod host;
pub mod ipc;
mod runtime;
mod settings;

pub use runtime::{Lifecycle, Node};
pub use settings::Settings;
