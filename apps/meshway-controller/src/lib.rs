//! The Meshway Controller application and lifecycle owners.
//!
//! Domain services and private persistence stay in this application. Shared
//! configuration, model, and wire types are owned by their respective crates.

#![forbid(unsafe_code)]

pub mod accounts;
mod app;
mod directory;
pub mod http;
mod secrets;
mod settings;
mod store;

pub use app::{App, AppError};
pub use settings::Settings;
