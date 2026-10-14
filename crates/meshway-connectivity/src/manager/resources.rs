//! Manager-owned physical workers. Intentional retirement is joined separately
//! from unexpected termination; neither creates another link or receive owner.
use super::*;
use meshway_model::state::ResourceRef;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Key {
    Listener(String),
    Candidate(String, bool),
    Peer(NodeId),
    Traversal,
    Idle,
    Facts,
}

pub(super) struct Owner {
    requested: CancellationToken,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
    socket: Option<[u8; 32]>,
}
impl Owner {
    fn stop(&self) {
        self.requested.cancel();
        self.stop.cancel();
    }
    pub(super) async fn join(mut self) -> Result<(), Error> {
        self.stop();
        if let Some(task) = self.task.take() {
            task.await.map_err(|_| Error::Worker)?;
        }
        Ok(())
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Manager {
    pub(super) fn spawn_owner<F: std::future::Future<Output = ()> + Send + 'static>(
        &mut self,
        key: Key,
        socket: Option<[u8; 32]>,
        build: impl FnOnce(Arc<Shared>) -> F,
    ) {
        debug_assert!(!self.owners.contains_key(&key));
        let shared = self.shared.for_owner();
        let stop = shared.stop.clone();
        let root = self.shared.stop.clone();
        let requested = CancellationToken::new();
        let expected = requested.clone();
        let future = build(shared);
        let task = tokio::spawn(async move {
            // Also fence a panic: the resource task's Drop is not an ordinary
            // retirement request and must surface at Manager::recv.
            struct Exit(CancellationToken, CancellationToken);
            impl Drop for Exit {
                fn drop(&mut self) {
                    if !self.0.is_cancelled() {
                        self.1.cancel();
                    }
                }
            }
            let _exit = Exit(expected, root);
            future.await;
        });
        self.owners.insert(
            key,
            Owner {
                requested,
                stop,
                task: Some(task),
                socket,
            },
        );
    }

    pub(super) fn retire_resources(&mut self, old: &Plan, new: &Plan) -> Result<Vec<Owner>, Error> {
        let compatible = |resource: &ResourceRef| old.netmap_compatible(new, resource);
        let dial_peers = new.dial_peers();
        let retired: Vec<_> = self
            .owners
            .keys()
            .filter(|key| !match key {
                Key::Listener(key) => compatible(&ResourceRef::Listener { key: key.clone() }),
                Key::Candidate(key, _) => compatible(&ResourceRef::Dial { key: key.clone() }),
                // A kept worker re-reads its peer's candidates per sweep.
                Key::Peer(peer) => dial_peers.contains(peer),
                Key::Traversal => true,
                Key::Idle | Key::Facts => true,
            })
            .cloned()
            .collect();
        Ok(self.retire_keys(&retired))
    }

    /// Stop the owners of `keys`; the caller joins them.
    pub(super) fn retire_keys(&mut self, keys: &[Key]) -> Vec<Owner> {
        keys.iter()
            .filter_map(|key| {
                if let Key::Listener(listener) = key
                    && let Ok(mut unbound) = self.shared.unbound.lock()
                {
                    unbound.remove(listener);
                }
                let owner = self.owners.remove(key)?;
                owner.stop();
                Some(owner)
            })
            .collect()
    }

    pub(super) async fn join_retired(&mut self, owners: Vec<Owner>) -> Result<(), Error> {
        for owner in owners {
            let socket = owner.socket;
            owner.join().await?;
            if let Some(id) = socket {
                self.listeners.retain(|fact| fact.socket_id != id);
                self.shared.remove_socket(id);
                self.shared
                    .traversal_endpoints
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .remove(&id);
                self.shared.resource_changes.send_replace(());
            }
        }
        // Every caller binds next; a retired owner's unadopted bind must not
        // still hold the port then.
        self.release_retired_binds()
    }
}
