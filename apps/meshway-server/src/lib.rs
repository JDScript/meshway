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
pub mod connectivity;
mod cursor;
mod directory;
pub mod effect;
pub mod enrollment;
pub mod error;
pub mod execution;
pub(crate) mod history;
pub mod http;
pub mod netmap;
pub mod networks;
mod secrets;
pub mod settings;
mod store;
mod stun;

pub use app::{App, AppError, Migrated};
pub use settings::{InvalidConsoleOrigin, Migration, Settings, console_origin};
