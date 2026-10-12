//! At most one pending sync per Node, woken by committed netmap changes, and
//! the leases issued to each Node, against which the lease a request names
//! is checked before it bounds that request's hold.
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
    issued: HashMap<Key, VecDeque<NetmapLease>>,
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

    /// End of `claimed` when this process issued exactly that lease to the
    /// Node; `None` otherwise (none named, forged, or issued before a
    /// restart), which answers without holding.
    pub(crate) fn held_until_ms(
        &self,
        network: NetworkId,
        node: NodeId,
        claimed: Option<&NetmapLease>,
    ) -> Option<u64> {
        let claimed = claimed?;
        self.state()
            .issued
            .get(&(network, node))
            .filter(|issued| issued.contains(claimed))
            .map(|_| claimed.expires_at_ms)
    }

    /// An answer carrying `lease` is about to be sent to the Node.
    pub(crate) fn issued(&self, network: NetworkId, node: NodeId, lease: NetmapLease) {
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
        if !issued.contains(&lease) {
            if issued.len() == MAX_ISSUED {
                issued.pop_front();
            }
            issued.push_back(lease);
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
        assert_eq!(hub.held_until_ms(network, node, None), None);
        assert_eq!(hub.held_until_ms(network, node, Some(&first)), None);
        hub.issued(network, node, first);
        // The answer carrying `second` is generated, then lost: the Node
        // retries naming `first`, which still bounds its hold.
        hub.issued(network, node, second);
        assert_eq!(hub.held_until_ms(network, node, Some(&first)), Some(1_000));
        assert_eq!(hub.held_until_ms(network, node, Some(&second)), Some(2_000));
        // A lease never issued (a longer end, another Node) bounds nothing.
        assert_eq!(
            hub.held_until_ms(network, node, Some(&lease(1, 9_000))),
            None
        );
        let other: NodeId = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        assert_eq!(hub.held_until_ms(network, other, Some(&first)), None);
        // Only the latest few are remembered.
        for end in 3..3 + MAX_ISSUED as u64 {
            hub.issued(network, node, lease(2, end * 1_000));
        }
        assert_eq!(hub.held_until_ms(network, node, Some(&first)), None);
    }
}
