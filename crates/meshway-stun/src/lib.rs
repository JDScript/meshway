//! Bounded, unauthenticated RFC 8489 Binding over UDP.
//!
//! A mapped address is only a path observation. Neither a transaction ID nor a
//! FINGERPRINT authenticates a peer, authorizes a route, or proves direct reachability.
//! The client codec never creates sockets: callers send on their protected peer
//! socket and retain ownership of endpoint selection, demultiplexing, retries and
//! deadlines. No TURN, credentials, RFC 3489, or TCP support is provided.
#![forbid(unsafe_code)]

mod codec;
mod service;

pub use codec::{
    BindingRequest, Error, Fingerprint, MAX_DATAGRAM_SIZE, MappedAddress, MessageKind,
    TransactionId, classify, encode_request, encode_response, parse_request, parse_response,
};
pub use service::{Service, ServiceConfig, ServiceConfigError, ServiceStats};
