//! Dynamic peer facts: the netmap's second line. State reports become per-Node
//! listener, STUN and exit-readiness facts in a volatile per-network ledger,
//! disclosed to each Node only within its current netmap projection. Facts
//! select among candidates the netmap already allows; they never authorize,
//! compile or publish anything.
mod extract;
#[cfg(test)]
mod tests;

pub(crate) use extract::peer_facts;

use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_configuration::netmap::{CompiledNetmap, NodeNetmap};
use meshway_model::{NetmapRef, NetworkId, NodeId, Sequence, StateEpoch};
use meshway_protocol::netmap::{PeerFacts, PeerFactsDelta, StateCursor};
use rusqlite::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

/// Current stored netmap of a Node, read in the caller's transaction. The
/// netmap store owner implements it; until it is installed nothing is visible.
pub trait NetmapReader: Send + Sync + 'static {
    fn current(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Option<Arc<CompiledNetmap>>, Error>;
}

/// No netmap store installed: every Node has no netmap, so no fact is
/// recorded and nothing is disclosed.
pub struct NoNetmaps;
impl NetmapReader for NoNetmaps {
    fn current(
        &self,
        _: &Transaction<'_>,
        _: NetworkId,
        _: NodeId,
    ) -> Result<Option<Arc<CompiledNetmap>>, Error> {
        Ok(None)
    }
}

/// Peer facts for the `sync` long poll. `held` is the netmap named in the
/// request: a cursor only continues within the netmap serial that scoped it.
pub trait PeerFactsSource: Send + Sync {
    /// Facts after `cursor` visible to `viewer`, or None when nothing changed.
    /// An unusable cursor (none, other epoch, other netmap, pruned) yields a
    /// complete replacement with a null base.
    fn peer_facts(
        &self,
        viewer: &CompiledNetmap,
        held: Option<NetmapRef>,
        cursor: Option<StateCursor>,
    ) -> Result<Option<PeerFactsDelta>, Error>;
    /// Register before computing facts: a change of any Node visible to
    /// `viewer` after this call resolves `FactsWake::changed`.
    fn watch(&self, viewer: &CompiledNetmap) -> Result<FactsWake, Error>;
}

/// Wakes a held `sync` when a visible peer's facts change or expire.
pub struct FactsWake {
    receiver: watch::Receiver<u64>,
    deadline: Option<DateTime<Utc>>,
}
impl FactsWake {
    pub fn deadline(&self) -> Option<DateTime<Utc>> {
        self.deadline
    }
    pub async fn changed(&mut self) {
        let sleep = async {
            match self.deadline {
                Some(at) => {
                    tokio::time::sleep((at - Utc::now()).to_std().unwrap_or_default()).await
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = self.receiver.changed() => {}
            _ = sleep => {}
        }
    }
    #[cfg(test)]
    pub(crate) fn has_changed(&self) -> bool {
        self.receiver.has_changed().unwrap_or(true)
    }
}

/// Bound on tombstones kept per network; older cursors get complete facts.
const MAX_REMOVED: usize = super::MAX_NETWORK_REPORTS;

struct Entry {
    facts: PeerFacts,
    sequence: Sequence,
}

struct NetworkFacts {
    sequence: Sequence,
    /// Cursors before this lost a pruned removal and need complete facts.
    floor: Sequence,
    current: BTreeMap<NodeId, Entry>,
    removed: BTreeMap<NodeId, Sequence>,
}

impl Default for NetworkFacts {
    fn default() -> Self {
        Self {
            sequence: Sequence::MIN,
            floor: Sequence::MIN,
            current: BTreeMap::new(),
            removed: BTreeMap::new(),
        }
    }
}

struct Waiter {
    sender: watch::Sender<u64>,
    visible: BTreeSet<NodeId>,
}

/// Volatile like the State hub: a restart changes the epoch, so every Node
/// re-reads complete facts.
pub(crate) struct FactLedger {
    epoch: StateEpoch,
    networks: BTreeMap<NetworkId, NetworkFacts>,
    waiters: BTreeMap<(NetworkId, NodeId), Waiter>,
}

/// Nodes visible to the netmap's owner: its projection, excluding itself.
pub fn visible(netmap: &NodeNetmap) -> BTreeSet<NodeId> {
    let own = netmap.node_id;
    netmap
        .projection
        .configuration
        .nodes
        .keys()
        .copied()
        .filter(|node| *node != own)
        .collect()
}

/// Endpoint details of a peer listener reach only a Node that may dial it.
fn dialable(viewer: &CompiledNetmap, peer: NodeId, listener: &str) -> bool {
    viewer
        .netmap
        .dialers
        .values()
        .any(|dial| dial.peer == peer && dial.listener_key == listener)
}

fn scoped(viewer: &CompiledNetmap, facts: &PeerFacts) -> PeerFacts {
    let mut facts = facts.clone();
    facts
        .listeners
        .retain(|key, _| dialable(viewer, facts.node_id, key));
    facts
}

impl FactLedger {
    pub(crate) fn new(epoch: StateEpoch) -> Self {
        Self {
            epoch,
            networks: BTreeMap::new(),
            waiters: BTreeMap::new(),
        }
    }

    fn next(network: &mut NetworkFacts) -> Result<Sequence, Error> {
        network.sequence = network
            .sequence
            .checked_next()
            .map_err(|_| Error::StateLimit)?;
        Ok(network.sequence)
    }

    fn wake(&mut self, network: NetworkId, node: NodeId) {
        self.waiters
            .retain(|_, waiter| waiter.sender.receiver_count() > 0);
        for ((candidate, _), waiter) in &self.waiters {
            if *candidate == network && waiter.visible.contains(&node) {
                waiter
                    .sender
                    .send_modify(|version| *version = version.wrapping_add(1));
            }
        }
    }

    /// Record a reporter's complete current facts. Unchanged content does not
    /// advance the cursor or wake anyone. Freshness is piggybacked on the next
    /// ordinary sync response. Returns whether semantic content advanced.
    pub(crate) fn record(
        &mut self,
        network: NetworkId,
        facts: PeerFacts,
        now: DateTime<Utc>,
    ) -> Result<bool, Error> {
        self.sweep(network, now)?;
        if facts.expires_at <= now {
            return self.remove(network, facts.node_id);
        }
        let node = facts.node_id;
        let state = self.networks.entry(network).or_default();
        if let Some(entry) = state.current.get_mut(&node)
            && entry.facts.listeners == facts.listeners
            && entry.facts.exit == facts.exit
            && facts.expires_at >= entry.facts.expires_at
        {
            entry.facts = facts;
            return Ok(false);
        }
        let sequence = Self::next(state)?;
        state.removed.remove(&node);
        state.current.insert(node, Entry { facts, sequence });
        self.wake(network, node);
        Ok(true)
    }

    /// The Node has no current facts: withdrawn, retired or without a netmap.
    pub(crate) fn remove(&mut self, network: NetworkId, node: NodeId) -> Result<bool, Error> {
        let Some(state) = self.networks.get_mut(&network) else {
            return Ok(false);
        };
        if state.current.remove(&node).is_none() {
            return Ok(false);
        }
        let sequence = Self::next(state)?;
        state.removed.insert(node, sequence);
        if state.removed.len() > MAX_REMOVED
            && let Some((oldest, at)) = state
                .removed
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(node, at)| (*node, *at))
        {
            state.removed.remove(&oldest);
            state.floor = at;
        }
        self.wake(network, node);
        Ok(true)
    }

    fn sweep(&mut self, network: NetworkId, now: DateTime<Utc>) -> Result<(), Error> {
        let expired: Vec<NodeId> = self.networks.get(&network).map_or_else(Vec::new, |state| {
            state
                .current
                .iter()
                .filter(|(_, entry)| entry.facts.expires_at <= now)
                .map(|(node, _)| *node)
                .collect()
        });
        for node in expired {
            self.remove(network, node)?;
        }
        Ok(())
    }

    pub(crate) fn delta(
        &mut self,
        viewer: &CompiledNetmap,
        held: Option<NetmapRef>,
        cursor: Option<StateCursor>,
        now: DateTime<Utc>,
    ) -> Result<Option<PeerFactsDelta>, Error> {
        let network = viewer.netmap.network_id;
        self.sweep(network, now)?;
        let visible = visible(&viewer.netmap);
        let empty = NetworkFacts::default();
        let state = self.networks.get(&network).unwrap_or(&empty);
        let next = StateCursor {
            epoch: self.epoch,
            sequence: state.sequence,
        };
        let base = cursor.filter(|cursor| {
            held == Some(viewer.reference())
                && cursor.epoch == self.epoch
                && cursor.sequence >= state.floor
                && cursor.sequence <= state.sequence
        });
        let after = base.map(|base| base.sequence);
        let freshness_only = after == Some(state.sequence);
        let updated: BTreeMap<_, _> = state
            .current
            .iter()
            .filter(|(node, entry)| {
                visible.contains(node)
                    && (freshness_only || after.is_none_or(|after| entry.sequence > after))
            })
            .map(|(node, entry)| (*node, scoped(viewer, &entry.facts)))
            .collect();
        let removed: Vec<_> = match after {
            None => Vec::new(),
            Some(after) => state
                .removed
                .iter()
                .filter(|(node, at)| visible.contains(node) && **at > after)
                .map(|(node, _)| *node)
                .collect(),
        };
        if base.is_some() && updated.is_empty() && removed.is_empty() {
            return Ok(None);
        }
        Ok(Some(PeerFactsDelta {
            base,
            cursor: next,
            netmap_serial: viewer.netmap.serial,
            updated,
            removed,
        }))
    }

    pub(crate) fn watch(&mut self, viewer: &CompiledNetmap) -> FactsWake {
        let network = viewer.netmap.network_id;
        let visible = visible(&viewer.netmap);
        let deadline = self.networks.get(&network).and_then(|state| {
            state
                .current
                .iter()
                .filter(|(node, _)| visible.contains(node))
                .map(|(_, entry)| entry.facts.expires_at)
                .min()
        });
        self.waiters
            .retain(|_, waiter| waiter.sender.receiver_count() > 0);
        let waiter = self
            .waiters
            .entry((network, viewer.netmap.node_id))
            .or_insert_with(|| Waiter {
                sender: watch::channel(0).0,
                visible: BTreeSet::new(),
            });
        waiter.visible = visible;
        FactsWake {
            receiver: waiter.sender.subscribe(),
            deadline,
        }
    }
}

/// Production `PeerFactsSource` over the State runtime's ledger.
#[derive(Clone)]
pub struct StateFacts {
    runtime: Arc<Mutex<super::service::Runtime>>,
}
impl StateFacts {
    pub(crate) fn new(runtime: Arc<Mutex<super::service::Runtime>>) -> Self {
        Self { runtime }
    }
}
impl PeerFactsSource for StateFacts {
    fn peer_facts(
        &self,
        viewer: &CompiledNetmap,
        held: Option<NetmapRef>,
        cursor: Option<StateCursor>,
    ) -> Result<Option<PeerFactsDelta>, Error> {
        let mut runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
        if runtime.is_closed() {
            return Err(Error::CLOSED);
        }
        runtime.facts.delta(viewer, held, cursor, Utc::now())
    }
    fn watch(&self, viewer: &CompiledNetmap) -> Result<FactsWake, Error> {
        let mut runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
        if runtime.is_closed() {
            return Err(Error::CLOSED);
        }
        Ok(runtime.facts.watch(viewer))
    }
}

/// The `sync` side of the same ledger: the viewer is the Node's current
/// stored netmap, read in the sync transaction.
impl crate::netmap::PeerFactsSource for StateFacts {
    fn read(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        node: NodeId,
        netmap: NetmapRef,
        held: Option<NetmapRef>,
        cursor: Option<StateCursor>,
    ) -> Result<crate::netmap::PeerFactsRead, Error> {
        let netmaps = {
            let runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
            Arc::clone(&runtime.netmaps)
        };
        let viewer = netmaps
            .current(tx, network, node)?
            .filter(|viewer| viewer.reference() == netmap)
            .ok_or(Error::BindingConflict)?;
        // Armed before reading, so no change in between is missed.
        let mut wake = PeerFactsSource::watch(self, &viewer)?;
        let delta = PeerFactsSource::peer_facts(self, &viewer, held, cursor)?;
        Ok(crate::netmap::PeerFactsRead {
            delta,
            changed: Some(Box::pin(async move { wake.changed().await })),
        })
    }
}
