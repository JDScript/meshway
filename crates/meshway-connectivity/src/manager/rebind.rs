//! Listener binding and its bounded retry. A listener whose local address is
//! unavailable (another process holds the port for a moment, the address is
//! not assigned yet, a privileged port without the capability) does not keep
//! the Manager from installing everything else, nor stays unbound until the
//! Node restarts: its own owner retries the bind under the current content,
//! starting after 1 s and doubling to 60 s (60 s throughout for a cause that
//! needs an operator), and reports why it is unavailable until it binds. A
//! path change (an address may have come back) runs the next attempt at once
//! and starts the backoff over.
use super::*;
use std::time::Duration;

const FIRST: Duration = Duration::from_secs(1);
const MAX: Duration = Duration::from_secs(60);

/// A configured listener of the current content that is not bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerUnavailable {
    pub key: String,
    /// The address the last attempt could not bind, when the bind was reached.
    pub address: Option<SocketAddr>,
    /// The failure of the last attempt, e.g. `listener lab-udp cannot bind
    /// 0.0.0.0:35000: Address in use (os error 98)`.
    pub detail: String,
    /// Whether the cause can end on its own (a held port, an address not yet
    /// assigned); otherwise it needs an operator. Both are retried.
    pub recoverable: bool,
    /// Failed attempts so far.
    pub attempts: u32,
    /// When the next attempt runs.
    pub retry_at: std::time::Instant,
}

pub(super) struct Unbound {
    /// The retry owner this entry belongs to; a bind it sends is adopted only
    /// while it is still that key's owner.
    token: u64,
    pub report: ListenerUnavailable,
}

/// A bound listener resource, not yet serving.
pub(super) enum Bound {
    Stream {
        fact: ListenerFact,
        listener: Box<dyn crate::ListenerSocket>,
        acceptor: Arc<provider::Acceptor>,
    },
    Datagram {
        fact: ListenerFact,
        endpoint: Box<dyn meshway_transport::raw::Datagram>,
    },
    Traversal {
        fact: ListenerFact,
        reference: meshway_protocol::connectivity::SocketReference,
        socket: crate::TraversalSocket,
    },
}

pub(super) struct Rebound {
    token: u64,
    key: String,
    bound: Bound,
}

fn recoverable(error: &Error) -> bool {
    !matches!(
        error,
        Error::Bind { source, .. } if source.kind() == std::io::ErrorKind::PermissionDenied
    )
}

/// One unbound listener's recovery: a cause that can end on its own is
/// retried after 1 s, doubling to 60 s, without a total limit; one that
/// needs an operator is proven and read (bound) again every 60 s.
pub(super) fn recovery() -> meshway_model::recovery::Recovery {
    meshway_model::recovery::Recovery::new(meshway_model::recovery::Policy {
        backoff: meshway_model::Backoff::new(FIRST, MAX),
        horizon: meshway_model::recovery::Horizon::Unbounded,
        reread_blocked: Some(MAX),
    })
}

/// Record one failed bind: when it runs again.
fn delay(recovery: &mut meshway_model::recovery::Recovery, error: &Error) -> Duration {
    let observation = if recoverable(error) {
        meshway_model::recovery::Observation::unknown("listener_unavailable", error)
    } else {
        meshway_model::recovery::Observation::proven("listener_permission", error)
    };
    let now = std::time::Instant::now();
    match recovery.observe(observation, now) {
        meshway_model::recovery::Step::RetryAt(at)
        | meshway_model::recovery::Step::Block(Some(at)) => at.saturating_duration_since(now),
        _ => MAX,
    }
}

/// Bind one listener of the current content. Native UDP binds as a
/// coordination socket when a live instance qualifies its State reference.
pub(super) async fn bind(
    shared: &Shared,
    factory: &Arc<dyn SocketFactory>,
    traversal: bool,
    key: &str,
) -> Result<Bound, Error> {
    let plan = shared.plan();
    let limits = shared.limits;
    let declaration = plan.listeners.get(key).ok_or(Error::Configuration)?;
    let (protocol, _) = provider::declaration(declaration);
    if protocol == TransportProtocol::NativeUdp {
        let id = adjacency::nonce()?;
        let reference = match traversal {
            true => traversal::socket_reference(
                shared,
                id,
                meshway_model::state::ResourceRef::Listener {
                    key: key.to_owned(),
                },
            )?,
            false => None,
        };
        if let Some(reference) = reference {
            let socket = tokio::time::timeout(
                limits.handshake_timeout,
                factory.traversal_listener(&plan.desired, key, reference.clone()),
            )
            .await
            .map_err(|_| Error::Cancelled)??;
            let fact = ListenerFact {
                socket_id: id,
                key: key.to_owned(),
                protocol,
                local: socket.endpoint.local_addr()?,
                buffers: socket.endpoint.buffers(),
            };
            log_buffers(&format!("listener {key}"), fact.local, fact.buffers);
            return Ok(Bound::Traversal {
                fact,
                reference,
                socket,
            });
        }
        let endpoint = tokio::time::timeout(
            limits.handshake_timeout,
            factory.datagram_listener(&plan.desired, key),
        )
        .await
        .map_err(|_| Error::Cancelled)??;
        let fact = ListenerFact {
            socket_id: adjacency::nonce()?,
            key: key.to_owned(),
            protocol,
            local: endpoint.local_addr()?,
            buffers: endpoint.buffers(),
        };
        log_buffers(&format!("listener {key}"), fact.local, fact.buffers);
        return Ok(Bound::Datagram { fact, endpoint });
    }
    let acceptor = Arc::new(provider::Acceptor::new(declaration)?);
    let listener =
        tokio::time::timeout(limits.handshake_timeout, factory.listen(&plan.desired, key))
            .await
            .map_err(|_| Error::Cancelled)??;
    let fact = ListenerFact {
        socket_id: adjacency::nonce()?,
        key: key.to_owned(),
        protocol,
        local: listener.local_addr()?,
        buffers: None,
    };
    Ok(Bound::Stream {
        fact,
        listener,
        acceptor,
    })
}

impl Shared {
    /// `recovery` holds the failures since the backoff last started over;
    /// `attempts` counts all of them.
    fn record_unbound(
        &self,
        token: u64,
        key: &str,
        error: &Error,
        attempts: u32,
        recovery: &mut meshway_model::recovery::Recovery,
    ) -> Duration {
        let recoverable = recoverable(error);
        let wait = delay(recovery, error);
        let report = ListenerUnavailable {
            key: key.to_owned(),
            address: match error {
                Error::Bind { address, .. } => Some(*address),
                _ => None,
            },
            detail: error.to_string(),
            recoverable,
            attempts,
            retry_at: std::time::Instant::now() + wait,
        };
        tracing::warn!(
            key,
            error = %report.detail,
            recoverable,
            attempts,
            retry_ms = wait.as_millis() as u64,
            "Listener unavailable; retrying its bind"
        );
        if let Ok(mut unbound) = self.unbound.lock() {
            unbound.insert(key.to_owned(), Unbound { token, report });
        }
        wait
    }
}

/// The retry owner of one unbound listener. It hands a bound resource to the
/// Manager and ends when the Manager adopts it or retires the key.
async fn retry(
    shared: Arc<Shared>,
    factory: Arc<dyn SocketFactory>,
    traversal: bool,
    token: u64,
    key: String,
    (mut wait, mut recovery): (Duration, meshway_model::recovery::Recovery),
    rebound: mpsc::Sender<Rebound>,
) {
    let mut attempts = 1;
    let mut roams = shared.roams.subscribe();
    roams.borrow_and_update();
    loop {
        tokio::select! {
            _ = shared.stop.cancelled() => return,
            _ = tokio::time::sleep(wait) => {}
            Ok(()) = roams.changed() => {
                roams.borrow_and_update();
                // An address may have come back: the backoff starts over.
                recovery = self::recovery();
            }
        }
        match bind(&shared, &factory, traversal, &key).await {
            Ok(bound) => {
                tokio::select! {
                    _ = shared.stop.cancelled() => {}
                    _ = rebound.send(Rebound { token, key, bound }) => {}
                }
                // An owner that ends unasked stops the Manager: this one
                // ends when the Manager adopts the bind (or retires the key).
                shared.stop.cancelled().await;
                return;
            }
            Err(error) => {
                attempts += 1;
                wait = shared.record_unbound(token, &key, &error, attempts, &mut recovery);
            }
        }
    }
}

impl Manager {
    /// The first bind of `key` failed: report it and start its retry owner.
    pub(super) fn retry_listener(&mut self, key: String, error: &Error) -> Result<(), Error> {
        self.rebind_tokens = self.rebind_tokens.checked_add(1).ok_or(Error::Capacity)?;
        let token = self.rebind_tokens;
        let mut recovery = recovery();
        let wait = self
            .shared
            .record_unbound(token, &key, error, 1, &mut recovery);
        let factory = self.factory.clone();
        let traversal = self.traversal.is_some();
        let sender = self.rebound_sender.clone();
        let owner_key = key.clone();
        self.spawn_owner(resources::Key::Listener(owner_key), None, |shared| {
            retry(
                shared,
                factory,
                traversal,
                token,
                key,
                (wait, recovery),
                sender,
            )
        });
        Ok(())
    }

    /// Install a listener its retry owner bound, if that owner still owns
    /// the key under the content it was bound for. The bind may predate the
    /// current instance (`set_instance` only sees installed listeners), so it
    /// is first made what a bind now would be ([`Self::current`]).
    pub(super) async fn adopt(&mut self, rebound: Rebound) -> Result<bool, Error> {
        let Rebound { token, key, bound } = rebound;
        {
            let mut unbound = self.shared.unbound.lock().map_err(|_| Error::Worker)?;
            if unbound.get(&key).is_none_or(|entry| entry.token != token) {
                return Ok(false);
            }
            unbound.remove(&key);
        }
        // Its retry owner only waits for this; dropping it is the request.
        self.owners.remove(&resources::Key::Listener(key.clone()));
        let bound = match self.current(&key, bound).await {
            Ok(bound) => bound,
            Err(error) => {
                self.retry_listener(key, &error)?;
                return Ok(true);
            }
        };
        let mut endpoints = BTreeMap::new();
        self.install_bound(vec![bound], &mut endpoints)?;
        self.publish_endpoints(endpoints)?;
        tracing::info!(key, "Listener bound after retry");
        Ok(true)
    }

    /// `bound` as a bind under the current instance would make it: a plain
    /// Native UDP listener becomes a coordination socket once an instance
    /// qualifies its reference (and back without one), and a coordination
    /// socket's reference and Underlay authorization name the current
    /// instance. A changed mode drops the old socket before binding again, so
    /// the new bind never meets its own port.
    async fn current(&self, key: &str, bound: Bound) -> Result<Bound, Error> {
        let traversal = self.traversal.is_some();
        let resource = || meshway_model::state::ResourceRef::Listener {
            key: key.to_owned(),
        };
        match bound {
            Bound::Datagram { fact, endpoint }
                if traversal
                    && traversal::socket_reference(&self.shared, fact.socket_id, resource())?
                        .is_some() =>
            {
                drop(endpoint);
                bind(&self.shared, &self.factory, traversal, key).await
            }
            Bound::Traversal {
                fact,
                reference,
                socket,
            } => match traversal::socket_reference(&self.shared, fact.socket_id, resource())? {
                Some(current) => {
                    if current != reference {
                        socket.authorization.rebind(&current)?;
                    }
                    Ok(Bound::Traversal {
                        fact,
                        reference: current,
                        socket,
                    })
                }
                None => {
                    drop(socket);
                    bind(&self.shared, &self.factory, traversal, key).await
                }
            },
            bound => Ok(bound),
        }
    }

    /// A bind its retry owner handed over but the Manager has not adopted
    /// yet holds the listener's port. Once that owner is retired (its key
    /// left the content, its declaration changed, or the Manager stops) the
    /// bind is closed here, synchronously, before anything binds again; a
    /// bind whose owner still owns the key stays queued for [`Self::adopt`].
    pub(super) fn release_retired_binds(&mut self) -> Result<(), Error> {
        let mut owned = Vec::new();
        while let Ok(rebound) = self.rebound.try_recv() {
            let current = self
                .shared
                .unbound
                .lock()
                .map_err(|_| Error::Worker)?
                .get(&rebound.key)
                .is_some_and(|entry| entry.token == rebound.token);
            if current {
                owned.push(rebound);
            }
        }
        // At most one bind per retry owner, at most one owner per listener:
        // the queue (one slot per listener) has room for every one.
        for rebound in owned {
            self.rebound_sender
                .try_send(rebound)
                .map_err(|_| Error::Worker)?;
        }
        Ok(())
    }

    /// Configured listeners of the current content that are not bound.
    pub fn unavailable_listeners(&self) -> Vec<ListenerUnavailable> {
        self.shared
            .unbound
            .lock()
            .map(|unbound| unbound.values().map(|entry| entry.report.clone()).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_back_off_from_one_second_to_a_minute() {
        let waits = |error: &Error, n: usize| -> Vec<u64> {
            let mut recovery = recovery();
            (0..n)
                .map(|_| delay(&mut recovery, error).as_secs())
                .collect()
        };
        let busy = Error::Bind {
            key: "k".into(),
            address: "0.0.0.0:35000".parse().unwrap(),
            source: std::io::ErrorKind::AddrInUse.into(),
        };
        let denied = Error::Bind {
            key: "k".into(),
            address: "0.0.0.0:443".parse().unwrap(),
            source: std::io::ErrorKind::PermissionDenied.into(),
        };
        assert_eq!(waits(&busy, 9), [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(waits(&denied, 2), [60, 60]);
        assert!(recoverable(&busy));
        assert!(!recoverable(&denied));
        assert!(recoverable(&Error::Underlay));
    }
}
