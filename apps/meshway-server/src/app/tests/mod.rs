//! Whole-application tests: real App, storage and services, no fake Node or
//! execution payloads. They inspect private storage through the executor
//! handle, which is why they live in the crate rather than in `tests/`.
use super::*;

mod commit;
mod connectivity;
mod lifecycle;
mod netmap;
mod node;
mod resources;
mod session;
