//! One consistent sync answer and the peer-fact source it consults.
use crate::error::Error;
use crate::secrets::Sealer;
use crate::store::{execution as execution_store, netmap as store};
use meshway_configuration::netmap::netmap_delta;
use meshway_model::{InstanceId, NetmapLease, NetmapRef, NetworkId, NodeId};
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
    /// Wake-up for facts changing after this read; null when facts never change.
    pub changed: Option<PeerFactsChanged>,
}

/// Owner of dynamic peer facts. Facts never authorize: the source only scopes
/// them by the visibility of the Node's current netmap.
pub trait PeerFactsSource: Send + Sync + 'static {
    /// Called inside the sync read transaction after the Node's credentials
    /// and current netmap were checked. `changed` must be armed before the
    /// facts are read, so no change between the two is missed. A returned
    /// delta must continue `cursor` (or have a null base) and carry
    /// `netmap.serial`.
    fn read(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        netmap: NetmapRef,
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
        _: Option<StateCursor>,
    ) -> Result<PeerFactsRead, Error> {
        Ok(PeerFactsRead {
            delta: None,
            changed: None,
        })
    }
}

/// Credentials, the netmap part, peer facts and a fresh lease from one transaction.
pub(super) fn read(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: &SyncRequest,
    facts: &dyn PeerFactsSource,
    validity: Duration,
) -> Result<(SyncResponse, Option<PeerFactsChanged>), Error> {
    let now = execution_store::now()?;
    let principal = execution_store::authorize_node(tx, sealer, boot, token, now)?;
    let (network, node) = (principal.network, principal.node);
    let current = store::current(tx, network, node)?.ok_or(Error::BindingConflict)?;
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
    let PeerFactsRead { delta, changed } =
        facts.read(tx, network, node, current.current, request.state)?;
    let now_ms = u64::try_from(now).map_err(|_| Error::Internal("system clock before epoch"))?;
    let validity_ms =
        u64::try_from(validity.as_millis()).map_err(|_| Error::Internal("netmap validity"))?;
    let lease = NetmapLease::new(current.current, now_ms, now_ms.saturating_add(validity_ms))
        .map_err(|_| Error::Internal("netmap validity"))?;
    let response = SyncResponse {
        lease,
        netmap,
        peer_facts: delta,
    };
    response
        .check(request)
        .map_err(|_| Error::Internal("inconsistent sync response"))?;
    Ok((response, changed))
}
