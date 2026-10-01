//! Internal P2b domain; not an HTTP contract and not installed by Store::open.
// Public within the controller for the future integration, currently deliberately unused.
#![allow(dead_code)]

mod auth;
mod device;
mod model;
mod repository;

pub(crate) use auth::*;
#[allow(unused_imports)] // integration-facing types, no production consumer yet
pub(crate) use device::*;
pub(crate) use model::*;
pub(crate) use repository::*;

#[cfg(test)]
mod tests;
