//! Real Server/Control + real UDP descriptors with an isolated userspace NAT
//! address/filter adapter. Host routing and DNS are never modified.
use super::support as fixture;
use super::*;
use crate::{
    DatagramAuthorization, TraversalSocket,
    traversal::{Operation, Reply, TraversalGateway},
};
use meshway_control::{
    Control,
    connectivity::{AttemptScope, DiscoveryContext},
};
use meshway_model::{Sequence, state::*};
use meshway_protocol::connectivity::{CandidateReference, SocketReference};

mod support;
use support::*;
mod nat;
use nat::NatFactory;
mod acceptance;
mod observation;
mod resources;
use acceptance::accept_scheduling;
mod punch;
use punch::exercise_traversal;
mod revocation;
use revocation::revocation_recovery;

#[tokio::test]
async fn prepare_reserve_accepts_locally_before_next_query_and_requires_bilateral_active() {
    accept_scheduling(false, false).await;
}

#[tokio::test]
async fn uncertain_local_accept_retries_original_reserved_offer_without_extending_authority() {
    accept_scheduling(true, false).await;
}

#[tokio::test]
async fn invalidated_original_accept_slot_cannot_activate_after_uncertain_commit() {
    accept_scheduling(true, true).await;
}

#[tokio::test]
async fn empty_published_listeners_use_same_fd_stun_bilateral_punch_and_existing_noise_owner() {
    exercise_traversal(false).await;
}

#[tokio::test]
async fn healthy_adjacency_keeps_nat_observations_fresh_without_reacquisition() {
    exercise_traversal(true).await;
}

#[tokio::test]
async fn revoked_then_restored_access_acquires_fresh_authenticated_channels_and_circuit() {
    revocation_recovery(None).await;
}

#[tokio::test]
async fn restored_adjacency_uses_live_witness_after_lower_id_stale_indirect_witness() {
    revocation_recovery(Some(true)).await;
}

#[tokio::test]
async fn restored_adjacency_uses_live_witness_before_higher_id_stale_indirect_witness() {
    revocation_recovery(Some(false)).await;
}
