//! Bounded stream/datagram adjacency owner. Packet crypto, flows and routing belong to
//! meshway-dataplane; socket protection and DNS belong to meshway-underlay.
#![forbid(unsafe_code)]

mod adjacency;
mod io;
mod manager;
mod provider;
mod scope;
pub mod traversal;

pub use io::{
    AcceptedSocket, DatagramAuthorization, DatagramSocket, ListenerSocket, SocketFactory,
    TraversalSocket,
};
pub use manager::{Event, LinkFact, ListenerFact, Manager, Snapshot, SocketFact};
pub use meshway_policy::admission::Carrier;

use meshway_model::state::{Capability, StreamKind, TransportRole};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Concurrent dial/handshake/channel owners. Each prepared listener may
    /// additionally own one just-accepted socket until capacity is checked.
    pub connections: usize,
    pub listeners: usize,
    pub candidates: usize,
    pub events: usize,
    pub handshake_timeout: Duration,
    pub retry_backoff: Duration,
    pub session: meshway_transport::Limits,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            connections: 64,
            listeners: 16,
            candidates: 128,
            events: 64,
            handshake_timeout: Duration::from_secs(5),
            retry_backoff: Duration::from_secs(1),
            session: meshway_transport::Limits::new(128 * 1024, 16, 512 * 1024, 16, 16)
                .expect("fixed valid session limits"),
        }
    }
}

impl Limits {
    fn validate(self) -> Result<(), Error> {
        if !(1..=256).contains(&self.connections)
            || !(1..=64).contains(&self.listeners)
            || !(1..=1024).contains(&self.candidates)
            || !(1..=1024).contains(&self.events)
            || self.handshake_timeout.is_zero()
            || self.handshake_timeout > Duration::from_secs(5)
            || self.retry_backoff < Duration::from_millis(100)
            || self.retry_backoff > Duration::from_secs(60)
        {
            return Err(Error::Limits);
        }
        Ok(())
    }
}

pub fn capabilities() -> BTreeMap<String, Capability> {
    [
        ("transport:native_udp", vec![StreamKind::Udp]),
        ("transport:native_tcp", vec![StreamKind::Tcp]),
        (
            "transport:vless",
            vec![StreamKind::Tcp, StreamKind::Websocket],
        ),
        ("transport:trojan", vec![StreamKind::Tcp]),
    ]
    .into_iter()
    .map(|(key, streams)| {
        (
            key.to_owned(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams,
            },
        )
    })
    .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid connectivity limits")]
    Limits,
    #[error("execution or adjacency authority expired")]
    Authority,
    #[error("unsupported transport configuration")]
    Configuration,
    #[error("transport operation cancelled or timed out")]
    Cancelled,
    #[error("invalid adjacency message or proof")]
    Authentication,
    #[error("bounded connection capacity exhausted")]
    Capacity,
    #[error("authenticated carrier is unavailable")]
    Unavailable,
    #[error("transport I/O failed")]
    Io(#[from] std::io::Error),
    #[error("protected socket operation failed")]
    Underlay,
    #[error("provider handshake failed")]
    Provider,
    #[error("packet transport failed")]
    Send(#[from] meshway_dataplane::carrier::Error),
    #[error("connectivity worker failed")]
    Worker,
}
