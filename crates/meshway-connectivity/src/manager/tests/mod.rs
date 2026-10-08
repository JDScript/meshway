use super::*;
use crate::{AcceptedSocket, DatagramSocket, ListenerSocket};
use futures_util::{future::BoxFuture, task::AtomicWaker};
use meshway_authority::ExecutionSnapshot;
use meshway_transport::raw::{Datagram, DatagramRead};
use meshway_transport_spec::{DialParameters, Listener};
use meshway_transport_udp::UdpTransport;
use std::{
    collections::BTreeSet,
    io,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

#[cfg(target_os = "linux")]
mod linux_resources;
mod local_endpoints;
mod reattachment;
mod regression;
mod resources;
mod streaming;
pub(super) mod support;
mod traversal;

use support::{Factory, Faults, frame, linked};

mod handshake;
mod receipts;
mod receive;
mod scope;

// Stable entry names are selected by the fast profile real-time exclusions.
#[tokio::test]
async fn udp_and_stream_receive_timestamp_precedes_delayed_consumer_poll() {
    receive::udp_and_stream_receive_timestamp_precedes_delayed_consumer_poll().await;
}

#[tokio::test]
async fn real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner() {
    handshake::real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner()
        .await;
}

#[tokio::test]
async fn simultaneous_configured_initiators_converge_on_one_logical_carrier() {
    handshake::simultaneous_configured_initiators_converge_on_one_logical_carrier().await;
}
