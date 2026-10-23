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
pub mod backup;
pub mod command;
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

pub use app::{App, AppError, Migrated, PERSISTENT_STORAGE_BLOCK};
pub use settings::{InvalidConsoleOrigin, Migration, Settings, console_origin};

pub use store::schema::migrate::{MigrationOutcome, outcome as migration_outcome};

/// Schema version of the databases this build writes.
pub const SCHEMA_VERSION: i64 = store::schema::VERSION;
/// The oldest schema version this build migrates from.
pub const MIN_MIGRATABLE_SCHEMA: i64 = store::schema::MIN_MIGRATABLE;
