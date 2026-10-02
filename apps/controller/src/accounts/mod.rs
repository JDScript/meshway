//! Local-account domain reused by Store transactions and the generated HTTP API.

mod auth;
mod device;
mod model;
mod repository;

pub(crate) use auth::*;
pub(crate) use device::*;
pub(crate) use model::*;
pub(crate) use repository::*;

#[cfg(test)]
mod tests;
