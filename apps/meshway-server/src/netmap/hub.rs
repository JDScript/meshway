//! At most one pending sync per Node, woken by committed netmap changes, and
//! the lease each Node holds, which bounds its next hold.
use crate::error::{Error, Unavailability};
use meshway_model::{NetworkId, NodeId};
use std::collections::{BTreeSet, HashMap};
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

/// At most this many Nodes' held leases are remembered; a forgotten Node's
/// next sync is answered at once, as after a Server restart.
const MAX_NODES: usize = 65_536;

/// A Node's latest sync request and the lease of its latest answer that no
/// newer request of the Node superseded (the lease the Node holds).
struct Node {
    ticket: u64,
    held_until_ms: Option<u64>,
}

#[derive(Default)]
pub(crate) struct Hub {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    waiters: HashMap<Key, (u64, oneshot::Sender<Wake>)>,
    nodes: HashMap<Key, Node>,
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
        let key = (network, node);
        if state.nodes.len() >= MAX_NODES
            && !state.nodes.contains_key(&key)
            && let Some(forgotten) = state
                .nodes
                .keys()
                .find(|other| !state.waiters.contains_key(*other))
                .copied()
        {
            state.nodes.remove(&forgotten);
        }
        let entry = state.nodes.entry(key).or_insert(Node {
            ticket,
            held_until_ms: None,
        });
        entry.ticket = ticket;
        let held_until_ms = entry.held_until_ms;
        Ok(Waiter {
            hub: Arc::clone(self),
            key,
            ticket,
            held_until_ms,
            receiver,
        })
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
    held_until_ms: Option<u64>,
    receiver: oneshot::Receiver<Wake>,
}

impl Waiter {
    /// Resolves once; a dropped sender counts as closed.
    pub(crate) async fn woken(&mut self) -> Wake {
        (&mut self.receiver).await.unwrap_or(Wake::Closed)
    }
    /// End of the lease the Node held when this request arrived; `None`
    /// when this Server process issued it none it is known to hold.
    pub(crate) fn held_until_ms(&self) -> Option<u64> {
        self.held_until_ms
    }
    /// This request is answered with a lease ending at `expires_at_ms`. A
    /// superseded request's answer is not one the Node holds.
    pub(crate) fn answered(&self, expires_at_ms: u64) {
        let mut state = self.hub.state();
        if let Some(node) = state.nodes.get_mut(&self.key)
            && node.ticket == self.ticket
        {
            node.held_until_ms = Some(expires_at_ms);
        }
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
        assert_eq!(third.held_until_ms(), None);
        assert_eq!(third.woken().await, Wake::Closed);
        assert!(hub.register(network, node).is_err());
    }

    #[test]
    fn only_an_answer_the_node_did_not_supersede_is_its_held_lease() {
        let hub = Arc::new(Hub::default());
        let (network, node) = ids();
        let first = hub.register(network, node).unwrap();
        assert_eq!(first.held_until_ms(), None);
        first.answered(1_000);
        drop(first);
        let old = hub.register(network, node).unwrap();
        assert_eq!(old.held_until_ms(), Some(1_000));
        // A newer request supersedes `old`: its later answer never reaches
        // the Node, which still holds the earlier lease.
        let new = hub.register(network, node).unwrap();
        old.answered(2_000);
        assert_eq!(new.held_until_ms(), Some(1_000));
        new.answered(3_000);
        drop((old, new));
        assert_eq!(
            hub.register(network, node).unwrap().held_until_ms(),
            Some(3_000)
        );
    }
}
