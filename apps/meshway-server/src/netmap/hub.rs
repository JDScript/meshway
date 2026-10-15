//! At most one pending sync per Node, woken by committed netmap changes, and
//! the answers issued to each Node: the lease a request names is checked
//! against them before it, and the peer facts that answer carried, bound
//! that request's hold.
use crate::error::{Error, Unavailability};
use meshway_model::{NetmapLease, NetworkId, NodeId};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wake {
    /// The Node's current netmap changed in a committed transaction.
    Changed,
    /// A newer request of the same Node took over.
    Replaced,
    /// The Server is shutting down.
    Closed,
}

type Key = (NetworkId, NodeId);

/// At most this many Nodes' issued leases are remembered; a forgotten Node's
/// next sync is answered at once, as after a Server restart.
const MAX_NODES: usize = 65_536;
/// Latest leases remembered per Node. A Node that lost more answers in a
/// row names a lease no longer known and is answered at once.
const MAX_ISSUED: usize = 8;

#[derive(Default)]
pub(crate) struct Hub {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    waiters: HashMap<Key, (u64, oneshot::Sender<Wake>)>,
    issued: HashMap<Key, VecDeque<Issued>>,
}

/// One answer issued to a Node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Issued {
    lease: NetmapLease,
    /// Earliest freshness bound among the peer facts it carried that a
    /// renewal can still keep fresh; see `netmap::facts_renewal_until_ms`.
    facts_until_ms: Option<u64>,
}

/// What the answer carrying a request's named lease left the Node holding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) lease_until_ms: u64,
    pub(crate) facts_until_ms: Option<u64>,
}

impl Hub {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Replaces any pending request of the same Node, which answers at once.
    pub(crate) fn register(
        self: &Arc<Self>,
        network: NetworkId,
        node: NodeId,
    ) -> Result<Waiter, Error> {
        let mut state = self.state();
        if state.closed {
            return Err(Error::Unavailable(Unavailability::Closed));
        }
        state.next += 1;
        let ticket = state.next;
        let (sender, receiver) = oneshot::channel();
        if let Some((_, previous)) = state.waiters.insert((network, node), (ticket, sender)) {
            let _ = previous.send(Wake::Replaced);
        }
        Ok(Waiter {
            hub: Arc::clone(self),
            key: (network, node),
            ticket,
            receiver,
        })
    }

    /// What the answer carrying `claimed` left the Node holding, when this
    /// process issued exactly that lease to the Node; `None` otherwise (none
    /// named, forged, or issued before a restart), which answers without
    /// holding.
    pub(crate) fn held(
        &self,
        network: NetworkId,
        node: NodeId,
        claimed: Option<&NetmapLease>,
    ) -> Option<Held> {
        let claimed = claimed?;
        self.state()
            .issued
            .get(&(network, node))?
            .iter()
            .find(|issued| issued.lease == *claimed)
            .map(|issued| Held {
                lease_until_ms: issued.lease.expires_at_ms,
                facts_until_ms: issued.facts_until_ms,
            })
    }

    /// An answer carrying `lease` and peer facts renewable until
    /// `facts_until_ms` is about to be sent to the Node.
    pub(crate) fn issued(
        &self,
        network: NetworkId,
        node: NodeId,
        lease: NetmapLease,
        facts_until_ms: Option<u64>,
    ) {
        let mut state = self.state();
        let key = (network, node);
        if state.issued.len() >= MAX_NODES
            && !state.issued.contains_key(&key)
            && let Some(forgotten) = state
                .issued
                .keys()
                .find(|other| !state.waiters.contains_key(*other))
                .copied()
        {
            state.issued.remove(&forgotten);
        }
        let issued = state.issued.entry(key).or_default();
        if let Some(known) = issued.iter_mut().find(|issued| issued.lease == lease) {
            // Two answers with one lease: the Node holds at least the earlier facts.
            known.facts_until_ms = match (known.facts_until_ms, facts_until_ms) {
                (Some(known), Some(new)) => Some(known.min(new)),
                (known, new) => known.or(new),
            };
        } else {
            if issued.len() == MAX_ISSUED {
                issued.pop_front();
            }
            issued.push_back(Issued {
                lease,
                facts_until_ms,
            });
        }
    }

    /// Called after COMMIT with the Nodes whose current netmap changed.
    pub(crate) fn wake(&self, network: NetworkId, nodes: &BTreeSet<NodeId>) {
        let mut state = self.state();
        for node in nodes {
            if let Some((_, sender)) = state.waiters.remove(&(network, *node)) {
                let _ = sender.send(Wake::Changed);
            }
        }
    }

    /// Answers every pending request and refuses new ones.
    pub(crate) fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        for (_, (_, sender)) in state.waiters.drain() {
            let _ = sender.send(Wake::Closed);
        }
    }

    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.state().waiters.len()
    }
}

/// A registered pending request; dropping it unregisters only itself.
pub(crate) struct Waiter {
    hub: Arc<Hub>,
    key: Key,
    ticket: u64,
    receiver: oneshot::Receiver<Wake>,
}

impl Waiter {
    /// Resolves once; a dropped sender counts as closed.
    pub(crate) async fn woken(&mut self) -> Wake {
        (&mut self.receiver).await.unwrap_or(Wake::Closed)
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        let mut state = self.hub.state();
        if state
            .waiters
            .get(&self.key)
            .is_some_and(|(ticket, _)| *ticket == self.ticket)
        {
            state.waiters.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (NetworkId, NodeId) {
        (
            "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            "00000000-0000-4000-8000-000000000002".parse().unwrap(),
        )
    }

    #[tokio::test]
    async fn one_waiter_per_node_and_only_changed_nodes_wake() {
        let hub = Arc::new(Hub::default());
        let (network, node) = ids();
        let mut first = hub.register(network, node).unwrap();
        let mut second = hub.register(network, node).unwrap();
        assert_eq!(first.woken().await, Wake::Replaced);
        drop(first);
        assert_eq!(hub.pending(), 1);
        let other: NodeId = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        hub.wake(network, &BTreeSet::from([other]));
        assert_eq!(hub.pending(), 1);
        hub.wake(network, &BTreeSet::from([node]));
        assert_eq!(second.woken().await, Wake::Changed);
        drop(second);
        assert_eq!(hub.pending(), 0);
        let mut third = hub.register(network, node).unwrap();
        hub.close();
        assert_eq!(third.woken().await, Wake::Closed);
        assert!(hub.register(network, node).is_err());
    }

    #[test]
    fn only_a_lease_this_process_issued_bounds_a_hold() {
        let hub = Arc::new(Hub::default());
        let (network, node) = ids();
        let lease = |serial: u64, end: u64| {
            NetmapLease::new(
                meshway_model::NetmapRef {
                    revision: meshway_model::Revision::MIN,
                    serial: meshway_model::Version::new(serial).unwrap(),
                    digest: meshway_model::Digest::from_bytes([serial as u8; 32]),
                },
                1,
                end,
            )
            .unwrap()
        };
        let (first, second) = (lease(1, 1_000), lease(1, 2_000));
        let until = |lease: &NetmapLease| Held {
            lease_until_ms: lease.expires_at_ms,
            facts_until_ms: None,
        };
        assert_eq!(hub.held(network, node, None), None);
        assert_eq!(hub.held(network, node, Some(&first)), None);
        hub.issued(network, node, first, None);
        // The answer carrying `second` is generated, then lost: the Node
        // retries naming `first`, which still bounds its hold, with the
        // facts that answer carried.
        hub.issued(network, node, second, Some(1_500));
        assert_eq!(hub.held(network, node, Some(&first)), Some(until(&first)));
        assert_eq!(
            hub.held(network, node, Some(&second)),
            Some(Held {
                lease_until_ms: 2_000,
                facts_until_ms: Some(1_500)
            })
        );
        // The same lease issued twice keeps the earlier facts bound.
        hub.issued(network, node, second, Some(1_800));
        assert_eq!(
            hub.held(network, node, Some(&second))
                .unwrap()
                .facts_until_ms,
            Some(1_500)
        );
        // A lease never issued (a longer end, another Node) bounds nothing.
        assert_eq!(hub.held(network, node, Some(&lease(1, 9_000))), None);
        let other: NodeId = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        assert_eq!(hub.held(network, other, Some(&first)), None);
        // Only the latest few are remembered.
        for end in 3..3 + MAX_ISSUED as u64 {
            hub.issued(network, node, lease(2, end * 1_000), None);
        }
        assert_eq!(hub.held(network, node, Some(&first)), None);
    }
}
