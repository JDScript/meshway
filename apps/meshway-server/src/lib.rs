//! The Meshway Server application and lifecycle owners.
//!
//! Domain services and private persistence stay in this application. Shared
//! configuration, model, and wire types are owned by their respective crates.

#![forbid(unsafe_code)]
pub mod observations;
pub mod resources;
pub mod state;

pub mod accounts;
mod app;
pub(crate) mod configuration;
mod directory;
pub mod enrollment;
pub mod error;
pub mod execution;
pub(crate) mod history;
pub mod http;
pub mod networks;
mod secrets;
mod settings;
mod store;

pub use app::{App, AppError};
pub use settings::Settings;
