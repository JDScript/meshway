//! Controller-generated HTTP contract and injectable client transport.
//! Generation is owned by the controller router and scripts/generate-protocol.mjs.
#[allow(dead_code, clippy::all)]
mod generated;
pub use generated::*;
