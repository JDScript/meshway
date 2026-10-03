//! Meshway HTTP and event contracts.
//!
//! Public envelopes reference the shared model and configuration types. Route
//! declarations live with their actual handlers in the Controller application.

#![forbid(unsafe_code)]

mod auth;
mod object;
mod problem;
mod secret;

pub use auth::{
    AuthOptionsResponse, AuthSessionResponse, LoginRequest, PasswordLengthUnit, PasswordRules,
    RegisterRequest, SessionIssuedResponse, SessionMetadata, UserView,
};
pub use problem::{EmptyParams, ErrorResponse, Problem};
pub use secret::WireSecret;
