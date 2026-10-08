use super::*;
use crate::adjacency::{Initiation, Kind, Transcript};
use meshway_dataplane::{
    Action, Packet,
    secure::{Received, SecureNode},
};

// A semantic recovery deadline below the unchanged six-second health window.
const RECOVERY: Duration = Duration::from_secs(3);

use super::support as fixture;
mod support;
use support::*;
pub(super) use support::{exchange, packet, packet_between};
mod publication;
mod recovery;
mod supersession;

// Keep exact fast-profile contract names while grouping their bodies.
#[tokio::test]
async fn passive_restart_challenges_dial_only_peer_without_retiring_it_until_fresh_ack_and_ready() {
    recovery::passive_restart_challenges_dial_only_peer_without_retiring_it_until_fresh_ack_and_ready().await;
}

#[tokio::test]
async fn native_tcp_session_survives_staggered_control_publications_without_reopening() {
    publication::native_tcp_session_survives_staggered_control_publications_without_reopening()
        .await;
}

#[tokio::test]
async fn staggered_three_node_managers_and_real_demand_publication_keep_original_circuit() {
    publication::staggered_three_node_managers_and_real_demand_publication_keep_original_circuit()
        .await;
}
