//! At most one pending sync per Node, woken by committed netmap changes.
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

#[derive(Default)]
pub(crate) struct Hub {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    next: u64,
    waiters: HashMap<Key, (u64, oneshot::Sender<Wake>)>,
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
}
