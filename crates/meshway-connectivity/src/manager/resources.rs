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
    /// This owner among every owner its key has had ([`Failed::token`]).
    token: u64,
}

/// What an owner's future ended with: `()` for an owner without a failure
/// of its own, the error for a listener socket owner.
pub(super) trait Outcome {
    fn failure(self) -> Option<Error>;
}
impl Outcome for () {
    fn failure(self) -> Option<Error> {
        None
    }
}
impl Outcome for Result<(), Error> {
    fn failure(self) -> Option<Error> {
        self.err()
    }
}

/// A listener socket owner that ended unasked with `error`: the Manager
/// replaces that one listener ([`Manager::listener_failed`]).
pub(super) struct Failed {
    key: String,
    token: u64,
    error: Error,
}

/// `error` and each of its sources, e.g. `transport I/O failed: Too many
/// open files (os error 24)`.
pub(super) fn chain(error: &Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

pub(super) fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// A listener socket owner's failures in a row (accept, receive). Each is
/// retried by that owner after 50 ms, doubling to 1 s; none ends it until
/// they have lasted [`HORIZON`] without one success. Then the owner ends
/// with the last error and the Manager rebinds that listener.
pub(super) struct Streak {
    since: Option<tokio::time::Instant>,
    failures: u32,
    wait: std::time::Duration,
}

const RETRY_FIRST: std::time::Duration = std::time::Duration::from_millis(50);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(1);
/// How long a listener socket may fail in a row before it is rebound.
pub(super) const HORIZON: std::time::Duration = std::time::Duration::from_secs(10);

impl Default for Streak {
    fn default() -> Self {
        Self {
            since: None,
            failures: 0,
            wait: RETRY_FIRST,
        }
    }
}

impl Streak {
    /// One more failure of `listener`'s `operation`: how long to wait
    /// before the next, or the error to end with once [`HORIZON`] passed.
    /// The first of a streak is a WARN with its cause, the rest debug.
    pub(super) fn failed(
        &mut self,
        listener: &str,
        operation: &'static str,
        error: Error,
    ) -> Result<std::time::Duration, Error> {
        let now = tokio::time::Instant::now();
        let since = *self.since.get_or_insert(now);
        self.failures = self.failures.saturating_add(1);
        let wait = self.wait;
        if self.failures == 1 {
            tracing::warn!(listener, operation, error = %chain(&error),
                retry_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                "Listener socket operation failed; retrying it");
        } else {
            tracing::debug!(listener, operation, error = %chain(&error),
                failures = self.failures,
                retry_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                "Listener socket operation still failing");
        }
        if now.saturating_duration_since(since) >= HORIZON {
            return Err(error);
        }
        self.wait = (wait * 2).min(RETRY_MAX);
        Ok(wait)
    }

    /// The operation succeeded: a streak in progress ends.
    pub(super) fn succeeded(&mut self, listener: &str, operation: &'static str) {
        if let Some(since) = self.since.take() {
            tracing::info!(
                listener,
                operation,
                failures = self.failures,
                failing_ms = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX),
                "Listener socket operation recovered"
            );
            *self = Self::default();
        }
    }
}
impl Owner {
    fn stop(&self) {
        self.requested.cancel();
        self.stop.cancel();
    }
    /// Joins the stopped owner within [`JOIN_GRACE`]; one still running then
    /// is aborted (its sockets and sessions drop with it) and reported as a
    /// failed join. A retirement or shutdown never waits on an owner forever.
    pub(super) async fn join(mut self) -> Result<(), Error> {
        self.stop();
        let Some(mut task) = self.task.take() else {
            return Ok(());
        };
        match tokio::time::timeout(JOIN_GRACE, &mut task).await {
            Ok(result) => result.map_err(|_| Error::Worker),
            Err(_) => {
                tracing::warn!(
                    grace_ms = u64::try_from(JOIN_GRACE.as_millis()).unwrap_or(u64::MAX),
                    "Connectivity owner did not stop within its grace; aborting it"
                );
                task.abort();
                let _ = task.await;
                Err(Error::Worker)
            }
        }
    }
}

/// How long a stopped owner has to end on its own before it is aborted.
pub(super) const JOIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Manager {
    /// Run one resource owner. An owner that ends without being asked to
    /// stops the whole Manager, saying why (its error, or its panic), except
    /// a listener socket owner that ends with an error: that is one
    /// listener's failure, and only that listener is replaced
    /// ([`Self::listener_failed`]).
    pub(super) fn spawn_owner<O, F>(
        &mut self,
        key: Key,
        socket: Option<[u8; 32]>,
        build: impl FnOnce(Arc<Shared>) -> F,
    ) where
        O: Outcome + Send + 'static,
        F: std::future::Future<Output = O> + Send + 'static,
    {
        use futures_util::FutureExt;
        debug_assert!(!self.owners.contains_key(&key));
        self.owner_tokens = self.owner_tokens.wrapping_add(1);
        let token = self.owner_tokens;
        let shared = self.shared.for_owner();
        let stop = shared.stop.clone();
        let requested = CancellationToken::new();
        let future = build(shared);
        // Also fence a panic or an abort: neither is an ordinary retirement
        // request, and each must surface at Manager::recv.
        struct Exit {
            requested: CancellationToken,
            root: CancellationToken,
            owner: String,
            reason: Option<String>,
        }
        impl Drop for Exit {
            fn drop(&mut self) {
                if self.requested.is_cancelled() {
                    return;
                }
                if !self.root.is_cancelled() {
                    tracing::warn!(owner = %self.owner,
                        reason = self.reason.as_deref().unwrap_or("dropped before it ended"),
                        "Connectivity owner ended unrequested; stopping the Manager");
                }
                self.root.cancel();
            }
        }
        let mut exit = Exit {
            requested: requested.clone(),
            root: self.shared.stop.clone(),
            owner: format!("{key:?}"),
            reason: None,
        };
        let listener = match &key {
            Key::Listener(listener) => Some((listener.clone(), self.failed_sender.clone())),
            _ => None,
        };
        let task = tokio::spawn(async move {
            let outcome = match std::panic::AssertUnwindSafe(future).catch_unwind().await {
                Ok(outcome) => outcome,
                Err(panic) => {
                    exit.reason = Some(format!("panicked: {}", panic_message(&*panic)));
                    drop(exit);
                    std::panic::resume_unwind(panic);
                }
            };
            if exit.requested.is_cancelled() {
                return;
            }
            exit.reason = Some(match (outcome.failure(), listener) {
                (Some(error), Some((key, failed))) => {
                    let reason = chain(&error);
                    let report = Failed {
                        key: key.clone(),
                        token,
                        error,
                    };
                    match failed.try_send(report) {
                        Ok(()) => {
                            tracing::warn!(listener = %key, error = %reason,
                                "Listener socket failed; rebinding only this listener");
                            // The Manager replaces it; nothing else stops.
                            exit.requested.cancel();
                            return;
                        }
                        Err(_) => format!("failed (its replacement could not be queued): {reason}"),
                    }
                }
                (Some(error), None) => format!("failed: {}", chain(&error)),
                (None, _) => "returned".to_owned(),
            });
            drop(exit);
        });
        self.owners.insert(
            key,
            Owner {
                requested,
                stop,
                task: Some(task),
                socket,
                token,
            },
        );
    }

    /// A listener socket owner failed ([`Failed`]): if it still owns its
    /// key, retire just that listener (its socket, its channels) and bind
    /// it again through its retry owner, which reports it unavailable until
    /// the bind succeeds. Every other owner keeps running.
    pub(super) async fn listener_failed(&mut self, failed: Failed) -> Result<bool, Error> {
        let key = Key::Listener(failed.key.clone());
        if self
            .owners
            .get(&key)
            .is_none_or(|owner| owner.token != failed.token)
        {
            return Ok(false);
        }
        let retired = self.retire_keys(std::slice::from_ref(&key));
        self.join_retired(retired).await?;
        self.retry_listener(failed.key, &failed.error)?;
        Ok(true)
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
                if let Key::Listener(listener) = key {
                    let _ = self.shared.unbound.with(|unbound| unbound.remove(listener));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// U4: a stopped owner that does not end (it waits on something its
    /// joiner holds) is aborted after its grace; the join ends, reported as
    /// failed, and does not wait forever. Red before: the join never ended.
    #[tokio::test]
    async fn an_owner_that_does_not_stop_is_aborted_after_its_grace() {
        let stop = CancellationToken::new();
        let owner = Owner {
            requested: CancellationToken::new(),
            stop: stop.clone(),
            task: Some(tokio::spawn(std::future::pending())),
            socket: None,
            token: 0,
        };
        let started = tokio::time::Instant::now();
        let joined = tokio::time::timeout(JOIN_GRACE * 2, owner.join())
            .await
            .expect("the join is bounded");
        assert!(matches!(joined, Err(Error::Worker)));
        assert!(started.elapsed() >= JOIN_GRACE);
        assert!(stop.is_cancelled());
    }
}
