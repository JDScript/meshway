//! One consistent sync answer and the peer-fact source it consults.
use crate::error::Error;
use crate::secrets::Sealer;
use crate::store::{execution as execution_store, netmap as store};
use meshway_configuration::netmap::netmap_delta;
use meshway_model::{InstanceId, NetmapLease, NetmapRef, NetworkId, NodeId};
use meshway_protocol::UdpTraversalRequests;
use meshway_protocol::netmap::{
    PeerFactsDelta, StateCursor, SyncNetmap, SyncRequest, SyncResponse,
};
use rusqlite::Transaction;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Resolves once after a peer fact visible to the reading Node may have changed.
pub type PeerFactsChanged = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Result of one peer-fact read.
pub struct PeerFactsRead {
    /// Facts after the cursor scoped to the current netmap; null when nothing is new.
    pub delta: Option<PeerFactsDelta>,
    /// Live traversal attempts that ask the Node to accept; see `SyncResponse`.
    pub traversal_requests: UdpTraversalRequests,
    /// Wake-up for facts or traversal requests changing after this read; null when they never change.
    pub changed: Option<PeerFactsChanged>,
}

/// Owner of dynamic peer facts. Facts never authorize: the source only scopes
/// them by the visibility of the Node's current netmap.
pub trait PeerFactsSource: Send + Sync + 'static {
    /// Called inside the sync read transaction after the Node's credentials
    /// and current netmap were checked. `changed` must be armed before the
    /// facts are read, so no change between the two is missed. A returned
    /// delta must continue `cursor` (or have a null base) and carry
    /// `netmap.serial`. `held` is the netmap the request named: a cursor
    /// continues only within the netmap that scoped it.
    fn read(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        netmap: NetmapRef,
        held: Option<NetmapRef>,
        cursor: Option<StateCursor>,
    ) -> Result<PeerFactsRead, Error>;
}

/// No peer facts: every answer has `peerFacts: null` and nothing wakes a hold.
pub struct NoPeerFacts;

impl PeerFactsSource for NoPeerFacts {
    fn read(
        &self,
        _: &Transaction<'_>,
        _: NetworkId,
        _: NodeId,
        _: NetmapRef,
        _: Option<NetmapRef>,
        _: Option<StateCursor>,
    ) -> Result<PeerFactsRead, Error> {
        Ok(PeerFactsRead {
            delta: None,
            traversal_requests: UdpTraversalRequests::default(),
            changed: None,
        })
    }
}

/// What every answer's lease is issued with.
pub(super) struct Issue<'a> {
    pub(super) signer: &'a super::signing::NetmapSigningKeys,
    pub(super) validity: Duration,
    pub(super) associations: &'a super::associations::Store,
}

/// Credentials, the netmap part, peer facts and a fresh lease from one transaction.
pub(super) fn read(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: &SyncRequest,
    facts: &dyn PeerFactsSource,
    issue: &Issue<'_>,
) -> Result<(SyncResponse, Option<PeerFactsChanged>), Error> {
    let now = execution_store::now()?;
    let principal = execution_store::authorize_node(tx, sealer, boot, token, now)?;
    let (network, node) = (principal.network, principal.node);
    let Some(current) = store::current(tx, network, node)? else {
        return Err(if store::unavailable(tx, network, node)? {
            Error::Unavailable(crate::error::Unavailability::Netmap)
        } else {
            // Authenticated, but its netmap was withdrawn (for example its
            // data-plane key was revoked): a decoded end of the held lease.
            Error::NetmapWithdrawn
        });
    };
    let netmap = if request.netmap == Some(current.current) {
        SyncNetmap::Unchanged
    } else if request.netmap.is_some() && request.netmap == current.previous {
        let base = store::load(
            tx,
            sealer,
            network,
            node,
            current.previous.ok_or(Error::Internal("netmap base"))?,
        )?;
        let target = store::load(tx, sealer, network, node, current.current)?;
        let delta = netmap_delta(&base, &target).map_err(|_| Error::Internal("netmap delta"))?;
        SyncNetmap::Delta {
            delta: Box::new(delta),
        }
    } else {
        SyncNetmap::Full {
            netmap: Box::new(store::load(tx, sealer, network, node, current.current)?),
        }
    };
    let PeerFactsRead {
        delta,
        traversal_requests,
        changed,
    } = facts.read(
        tx,
        network,
        node,
        current.current,
        request.netmap,
        request.state,
    )?;
    let now_ms = u64::try_from(now).map_err(|_| Error::Internal("system clock before epoch"))?;
    let validity_ms = u64::try_from(issue.validity.as_millis())
        .map_err(|_| Error::Internal("netmap validity"))?;
    let lease = NetmapLease::new(current.current, now_ms, now_ms.saturating_add(validity_ms))
        .map_err(|_| Error::Internal("netmap validity"))?;
    let (signature, key_rotation) = issue.signer.issue(tx, network, node, &lease, now)?;
    let overlay = issue
        .associations
        .overlay(tx, sealer, network, node, current.current, now_ms)?;
    let digest = overlay
        .as_ref()
        .map(|overlay| overlay.digest())
        .transpose()
        .map_err(|_| Error::Internal("association digest"))?;
    // Unchanged is null; a held overlay that ended or became invalid is
    // cleared by an empty one.
    let associations = if digest == request.associations {
        None
    } else {
        let overlay = overlay.unwrap_or(meshway_configuration::netmap::DnsAssociationOverlay {
            netmap: current.current,
            associations: Vec::new(),
        });
        let (signature, rotation_signature) = issue
            .signer
            .sign_associations(tx, network, node, &overlay, now)?;
        Some(meshway_protocol::DnsAssociationsUpdate {
            overlay,
            signature,
            rotation_signature,
        })
    };
    let response = SyncResponse {
        signature,
        lease,
        netmap,
        peer_facts: delta,
        key_rotation,
        traversal_requests,
        associations,
    };
    response
        .check(request)
        .map_err(|_| Error::Internal("inconsistent sync response"))?;
    Ok((response, changed))
}
