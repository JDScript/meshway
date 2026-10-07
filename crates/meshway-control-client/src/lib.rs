//! Generated Meshway operations over caller-owned HTTP transport.
#![forbid(unsafe_code)]

mod generated;
pub use generated::*;
mod runtime;
pub use runtime::{Client, ClientError, http_client_builder};
mod sse;
pub use sse::{EventDelivery, EventStream, StreamOptions, StreamResponse};
