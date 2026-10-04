//! Bounded DNS query ownership, wire handling and configuration selection.
//!
//! The control snapshot owns publication validity; the host owns socket routing.
//! A resolver candidate or a serialized answer grants no network access. This
//! crate delegates approved local resolver exchanges to the host. It does not
//! create trusted DNS associations, install system DNS, or cache data.
#![forbid(unsafe_code)]

mod dispatcher;
mod owner;
mod route;
mod selection;
mod service;
mod tcp;
mod wire;

pub use dispatcher::{
    DiscoveryGateway, DispatchError, Dispatcher, DnsFuture, GatewayError, OwnerDispatcher,
    ResolverExchange, Scope,
};
pub use owner::{
    Answer, Begin, DiscoveryRequest, Limits, Owner, OwnerError, Ticket, Unavailable, UpstreamWork,
};
pub use route::{LocalResolverRoute, RouteError};
pub use selection::{
    ConfigurationError, ExitSelection, PublishedAnswer, PublishedDns, ResolverCandidate, Selection,
    SelectionUnavailable,
};
pub use service::{Service, ServiceLimits};
pub use tcp::{PayloadCodec, TcpCodec, TcpError};
pub use wire::{
    Query, ResponseTransport, TrustedUpstreamResponse, WireError, decode_query, encode_response,
};
