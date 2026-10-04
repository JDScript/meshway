use crate::{
    Carrier, Error, Limits, SocketFactory, adjacency, provider,
    scope::{self, Scope},
};
use meshway_control::DataPlaneIdentity;
use meshway_dataplane::secure::PendingFrame;
use meshway_model::NodeId;
use meshway_policy::admission::admit_adjacency;
use meshway_transport::{Frame, Sender, Session};
use meshway_transport_spec::TransportProtocol;
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Semaphore, mpsc},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

mod workers;

#[derive(Clone, Debug)]
pub struct ListenerFact {
    pub key: String,
    pub protocol: TransportProtocol,
    pub local: SocketAddr,
}

/// Only produced after the three-message registered-key adjacency handshake.
/// `id` is channel-specific; it is not a logical IP circuit identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkFact {
    pub id: [u8; 32],
    pub peer: NodeId,
    pub carrier: Carrier,
    pub local: SocketAddr,
    pub remote: SocketAddr,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub listeners: Vec<ListenerFact>,
    pub links: Vec<LinkFact>,
}

#[derive(Debug)]
pub enum Event {
    /// Re-read actual current facts/carriers; no peer state report grants a link.
    Changed,
    Frame {
        link: Box<LinkFact>,
        frame: Frame,
    },
    Closed,
}

struct Link {
    fact: LinkFact,
    sender: Arc<AsyncMutex<Sender>>,
    scope: Scope,
}

struct Shared {
    identity: DataPlaneIdentity,
    stop: CancellationToken,
    links: Mutex<BTreeMap<NodeId, Link>>,
    events: mpsc::Sender<Event>,
    capacity: Arc<Semaphore>,
    limits: Limits,
}

impl Shared {
    fn scope(&self, edge: Option<(NodeId, Carrier)>, handshake: bool) -> Scope {
        let deadline: tokio::time::Instant = self.identity.snapshot().expires_at().into();
        Scope {
            snapshot: self.identity.snapshot().clone(),
            cancellation: self.stop.child_token(),
            deadline: if handshake {
                deadline.min(tokio::time::Instant::now() + self.limits.handshake_timeout)
            } else {
                deadline
            },
            edge,
        }
    }

    async fn event(&self, scope: &Scope, event: Event) -> Result<(), Error> {
        scope
            .run(async { self.events.send(event).await.map_err(|_| Error::Cancelled) })
            .await
    }
}

/// A publication-scoped collection of prepared listeners and bounded workers.
/// Reconcile closes physical streams before installing the new publication;
/// the host retains its one SecureNode/FlowTable for logical flow continuity.
pub struct Manager {
    factory: Arc<dyn SocketFactory>,
    shared: Arc<Shared>,
    listeners: Vec<ListenerFact>,
    events: mpsc::Receiver<Event>,
    workers: JoinSet<()>,
}

impl Manager {
    pub async fn start(
        identity: DataPlaneIdentity,
        factory: Arc<dyn SocketFactory>,
        limits: Limits,
    ) -> Result<Self, Error> {
        limits.validate()?;
        let cfg = identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        if cfg.listeners.len() > limits.listeners
            || cfg.dialers.len() > limits.candidates
            || cfg.peers.len() > limits.candidates
        {
            return Err(Error::Capacity);
        }
        let mut prepared = Vec::new();
        // All declarations must validate and bind before any worker is installed.
        for (key, declaration) in &cfg.listeners {
            let (protocol, enabled) = provider::declaration(declaration);
            if !enabled {
                continue;
            }
            let acceptor = Arc::new(provider::Acceptor::new(declaration)?);
            if key.is_empty() || key.len() > 128 {
                return Err(Error::Configuration);
            }
            let listener = tokio::select! { biased;
                _ = identity.snapshot().invalidated() => return Err(Error::Authority),
                result = tokio::time::timeout(limits.handshake_timeout, factory.listen(identity.snapshot(), key)) => result.map_err(|_| Error::Cancelled)??,
            };
            let fact = ListenerFact {
                key: key.clone(),
                protocol,
                local: listener.local_addr()?,
            };
            prepared.push((fact, listener, acceptor));
        }
        let plans = workers::plans(&identity, limits)?;
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let (tx, events) = mpsc::channel(limits.events);
        let shared = Arc::new(Shared {
            identity,
            stop: CancellationToken::new(),
            links: Mutex::new(BTreeMap::new()),
            events: tx,
            capacity: Arc::new(Semaphore::new(limits.connections)),
            limits,
        });
        let mut manager = Self {
            factory,
            shared,
            listeners: Vec::new(),
            events,
            workers: JoinSet::new(),
        };
        for (fact, listener, acceptor) in prepared {
            manager.listeners.push(fact.clone());
            manager.workers.spawn(workers::listen(
                manager.shared.clone(),
                fact,
                listener,
                acceptor,
            ));
        }
        for (peer, candidates) in plans {
            manager.workers.spawn(workers::dial(
                manager.shared.clone(),
                manager.factory.clone(),
                peer,
                candidates,
            ));
        }
        Ok(manager)
    }

    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        if self.shared.stop.is_cancelled() {
            return Err(Error::Unavailable);
        }
        self.shared
            .identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        Ok(Snapshot {
            listeners: self.listeners.clone(),
            links: links
                .values()
                .filter(|link| link.scope.check().is_ok())
                .map(|link| link.fact.clone())
                .collect(),
        })
    }

    pub fn carriers(&self) -> Result<BTreeMap<NodeId, Carrier>, Error> {
        Ok(self
            .snapshot()?
            .links
            .into_iter()
            .map(|link| (link.peer, link.carrier))
            .collect())
    }

    /// Never redirects or re-signs a PendingFrame. Exact signed carrier match is
    /// checked before queue admission and again after obtaining the send owner.
    pub async fn send(&self, pending: PendingFrame) -> Result<(), Error> {
        pending.ensure_current().map_err(|_| Error::Authority)?;
        let (sender, scope, fact) = {
            let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
            let link = links.get(&pending.next()).ok_or(Error::Unavailable)?;
            if &link.fact.carrier != pending.carrier() {
                return Err(Error::Unavailable);
            }
            (link.sender.clone(), link.scope.clone(), link.fact.clone())
        };
        scope
            .run(async {
                let mut sender = sender.lock().await;
                scope.check()?;
                let current = self
                    .shared
                    .links
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .get(&fact.peer)
                    .is_some_and(|link| link.fact.id == fact.id);
                if !current {
                    return Err(Error::Unavailable);
                }
                meshway_dataplane::carrier::send(&mut sender, pending, Some(fact.remote)).await?;
                Ok(())
            })
            .await
    }

    pub async fn recv(&mut self) -> Event {
        loop {
            let event = tokio::select! { biased;
                _ = self.shared.stop.cancelled() => return Event::Closed,
                _ = self.shared.identity.snapshot().invalidated() => return Event::Closed,
                _ = self.workers.join_next(), if !self.workers.is_empty() => {
                    self.shared.stop.cancel();
                    return Event::Closed;
                },
                event = self.events.recv() => match event { Some(event) => event, None => return Event::Closed },
            };
            if self.shared.stop.is_cancelled()
                || self.shared.identity.snapshot().ensure_current().is_err()
            {
                return Event::Closed;
            }
            if let Event::Frame { link, .. } = &event {
                let current = self.shared.links.lock().is_ok_and(|links| {
                    links.get(&link.peer).is_some_and(|current| {
                        current.fact.id == link.id && current.scope.check().is_ok()
                    })
                });
                if !current {
                    continue;
                }
            }
            return event;
        }
    }

    pub async fn reconcile(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        // Fresh snapshot guards are mandatory even for otherwise identical targets.
        self.shutdown().await?;
        let replacement = Self::start(identity, self.factory.clone(), self.shared.limits).await?;
        *self = replacement;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), Error> {
        self.shared.stop.cancel();
        let mut failed = false;
        while let Some(result) = self.workers.join_next().await {
            failed |= result.is_err();
        }
        self.shared.links.lock().map_err(|_| Error::Worker)?.clear();
        self.listeners.clear();
        self.events.close();
        while self.events.try_recv().is_ok() {}
        if failed { Err(Error::Worker) } else { Ok(()) }
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.shared.stop.cancel();
        // JoinSet drop aborts tasks; every socket/session remains owned by a task
        // or this map. Explicit shutdown is the asynchronous join boundary.
        if let Ok(mut links) = self.shared.links.lock() {
            links.clear();
        }
    }
}

fn rank(fact: &LinkFact) -> (NodeId, NodeId, &str, TransportProtocol, [u8; 32]) {
    (
        fact.carrier.dialer,
        fact.carrier.listener,
        &fact.carrier.listener_key,
        fact.carrier.protocol,
        fact.id,
    )
}

async fn connected(
    shared: Arc<Shared>,
    stream: Box<dyn meshway_transport::raw::Stream>,
    fact: LinkFact,
) -> Result<(), Error> {
    let scope = shared.scope(Some((fact.peer, fact.carrier.clone())), false);
    scope.check()?;
    let (sender, mut receiver) = Session::stream(
        scope::Stream {
            inner: stream,
            scope: scope.clone(),
        },
        fact.remote,
        shared.limits.session,
    )
    .into_split();
    let sender = Arc::new(AsyncMutex::new(sender));
    let selected = {
        let mut links = shared.links.lock().map_err(|_| Error::Worker)?;
        if links
            .get(&fact.peer)
            .is_some_and(|old| old.scope.check().is_ok() && rank(&old.fact) <= rank(&fact))
        {
            false
        } else {
            if let Some(old) = links.remove(&fact.peer) {
                old.scope.cancellation.cancel();
            }
            links.insert(
                fact.peer,
                Link {
                    fact: fact.clone(),
                    sender: sender.clone(),
                    scope: scope.clone(),
                },
            );
            true
        }
    };
    if !selected {
        sender
            .lock()
            .await
            .shutdown()
            .await
            .map_err(|_| Error::Worker)?;
        return Ok(());
    }
    let receive = async {
        shared.event(&scope, Event::Changed).await?;
        loop {
            scope.check()?;
            let frame = receiver.recv().await.map_err(|_| Error::Unavailable)?;
            shared
                .event(
                    &scope,
                    Event::Frame {
                        link: Box::new(fact.clone()),
                        frame,
                    },
                )
                .await?;
        }
    };
    let result = scope.run(receive).await;
    scope.cancellation.cancel();
    drop(receiver);
    {
        let mut links = shared.links.lock().map_err(|_| Error::Worker)?;
        if links
            .get(&fact.peer)
            .is_some_and(|link| link.fact.id == fact.id)
        {
            links.remove(&fact.peer);
        }
    }
    sender
        .lock()
        .await
        .shutdown()
        .await
        .map_err(|_| Error::Worker)?;
    let _ = shared
        .event(&shared.scope(None, false), Event::Changed)
        .await;
    result
}
