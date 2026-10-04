//! Bounded DNS query ownership, wire handling and configuration selection.
//!
//! The control snapshot owns publication validity; the host owns socket routing.
//! A resolver candidate or a serialized answer grants no network access. This
//! crate does not resolve upstreams, create trusted DNS associations or cache data.
#![forbid(unsafe_code)]

mod owner;
mod selection;
mod tcp;
mod wire;

pub use owner::{Answer, Begin, DiscoveryRequest, Limits, Owner, OwnerError, Ticket, Unavailable};
pub use selection::{
    ConfigurationError, ExitSelection, PublishedAnswer, PublishedDns, ResolverCandidate, Selection,
    SelectionUnavailable,
};
pub use tcp::{TcpCodec, TcpError};
pub use wire::{Query, ResponseTransport, WireError, decode_query, encode_response};
