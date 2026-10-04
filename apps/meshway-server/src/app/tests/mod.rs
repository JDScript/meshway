//! Whole-application tests: real App, storage and services, no fake Node or
//! execution payloads. They inspect private storage through the executor
//! handle, which is why they live in the crate rather than in `tests/`.
use super::*;

mod cache;
mod commit;
mod demand_renewal;
mod discovery;
mod lifecycle;
mod node;
mod refresh;
mod resources;
mod session;
mod state_scope;
