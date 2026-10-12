//! `POST /agent-api/v1/sync`: the single Node long poll.
//!
//! A call is prepared under the Control driver ([`Control::begin_sync`]), sent
//! without holding it ([`SyncCall::send`]) and applied under it again
//! ([`Control::finish_sync`]). The netmap part is checked against the request,
//! verified (full), reconstructed and digest-checked (delta) or matched to the
//! held content (unchanged), bound to this identity, saved and published to
//! Engine. Peer facts fold into Control's last known facts, which it
//! publishes to a [`PeerFactsSink`]; they never become netmap content.
use super::NetmapChange as Change;
use crate::{Control, Error};
use meshway_configuration::netmap::{CompiledNetmap, apply_netmap_delta};
use meshway_control_client::ClientError;
use meshway_model::NetmapRef;
use meshway_model::NodeId;
use meshway_protocol::{
    ErrorResponse, PeerFacts, SyncNetmap, SyncRequest, SyncResponse, ipc::SyncError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

/// Total budget of one sync request; above the Server's 60 s hold.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(90);
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(15);
/// Spacing after an immediate answer that changed nothing, so a Server that
/// does not hold cannot drive a tight loop.
const IDLE_SPACING: Duration = Duration::from_secs(1);

pub type SyncFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SyncResponse, ClientError<ErrorResponse>>> + Send + 'a>>;

/// One sync HTTP exchange with the Node session token. Without an explicit
/// transport Control uses the generated client operation for the sync route
/// on its own protected client, with a [`SYNC_TIMEOUT`] request budget.
pub trait SyncTransport: Send + Sync {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a>;
}

/// The generated `syncNode` operation on Control's own client.
pub(crate) struct ClientTransport(pub(crate) meshway_control_client::Client);

impl SyncTransport for ClientTransport {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a> {
        Box::pin(self.0.sync_node(request, token))
    }
}

/// The facts consumer refused this delta (e.g. its bounded queue is full).
/// Control then requests complete facts on the next sync.
/// Control's last known peer facts of the held netmap: one owner (Control)
/// publishes it; consumers read the latest value and keep no copy of their
/// own. Selection input only, never authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownPeers {
    pub netmap: NetmapRef,
    pub peers: Arc<BTreeMap<NodeId, PeerFacts>>,
    /// Live traversal attempts of the last answer that ask this Node to
    /// accept: a prompt to coordinate with the peer, never authority.
    pub traversal_requests: Arc<Vec<meshway_protocol::UdpTraversalRequest>>,
}

impl KnownPeers {
    /// Exits whose readiness fact is current, ready and has IPv4 installed.
    pub fn ready_exits(&self, now: chrono::DateTime<chrono::Utc>) -> BTreeSet<NodeId> {
        self.peers
            .iter()
            .filter(|(_, facts)| facts.expires_at > now)
            .filter(|(_, facts)| {
                facts.exit.as_ref().is_some_and(|exit| {
                    exit.status == meshway_model::exit::ExitStatus::Ready
                        && exit
                            .installed_families
                            .contains(&meshway_model::exit::ExitFamily::Ipv4)
                })
            })
            .map(|(peer, _)| *peer)
            .collect()
    }
}

/// Where Control publishes [`KnownPeers`] after they change.
pub trait PeerFactsSink {
    fn publish(&mut self, facts: KnownPeers);
}

impl PeerFactsSink for tokio::sync::watch::Sender<Option<KnownPeers>> {
    fn publish(&mut self, facts: KnownPeers) {
        self.send_replace(Some(facts));
    }
}

/// What a successful sync did to the held netmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetmapChange {
    /// New content (first installation or a larger serial).
    New { previous: Option<NetmapRef> },
    /// Same content with a new lease. Engine extends its timer, or installs
    /// the content again when the previous lease had already ended.
    Renewed,
}

pub struct SyncOutcome {
    pub netmap: meshway_engine::Netmap,
    pub change: NetmapChange,
    /// A facts delta was accepted by the sink.
    pub facts: bool,
}

/// A prepared request, independent of the Control borrow.
pub struct SyncCall {
    transport: Arc<dyn SyncTransport>,
    token: Zeroizing<String>,
    request: SyncRequest,
    exchange: Weak<()>,
}

/// The transport outcome of one call, to be applied by the same Control.
pub struct SyncReply {
    request: SyncRequest,
    exchange: Weak<()>,
    result: Result<SyncResponse, ClientError<ErrorResponse>>,
}

impl SyncCall {
    pub fn request(&self) -> &SyncRequest {
        &self.request
    }

    /// Bounded by [`SYNC_TIMEOUT`]; a timeout is a network failure.
    pub async fn send(self) -> SyncReply {
        let result = tokio::time::timeout(
            SYNC_TIMEOUT,
            self.transport.sync(&self.request, &self.token),
        )
        .await
        .unwrap_or(Err(ClientError::Network { status: None }));
        SyncReply {
            request: self.request,
            exchange: self.exchange,
            result,
        }
    }
}

impl Control {
    pub fn set_sync_transport(&mut self, transport: Arc<dyn SyncTransport>) {
        self.sync_transport = Some(transport);
    }

    /// Prepare one long poll naming the held netmap and facts cursor. Needs a
    /// live session only; no instance is required. Last known facts end with
    /// the netmap: once no lease is published (ended or withdrawn), the
    /// cursor ends too and this request asks for complete facts, which the
    /// Server answers without holding.
    pub fn begin_sync(&mut self) -> Result<SyncCall, Error> {
        let token = Zeroizing::new(self.token()?.to_owned());
        if self.netmap.facts.is_some() && self.netmap().is_none() {
            self.netmap.facts = None;
            self.netmap.associations = None;
            self.netmap.traversal = Arc::default();
            self.netmap.exchange = Arc::new(());
        }
        let transport = match &self.sync_transport {
            Some(transport) => transport.clone(),
            None => Arc::new(ClientTransport(
                self.client.clone().with_request_timeout(SYNC_TIMEOUT),
            )),
        };
        Ok(SyncCall {
            transport,
            token,
            request: self.sync_request(),
            exchange: Arc::downgrade(&self.netmap.exchange),
        })
    }

    fn sync_request(&self) -> SyncRequest {
        SyncRequest {
            netmap: self.netmap_reference(),
            state: self.netmap.facts,
            server_key_generation: self
                .record
                .binding
                .as_ref()
                .map(|binding| binding.server_signing_key.key_generation),
            traversal_requests: self
                .netmap
                .traversal
                .iter()
                .map(|request| request.attempt_id)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap_or_default(),
            associations: self.held_associations().map(|(digest, _)| digest),
        }
    }

    /// Apply one reply. A reply prepared before any later installation, stop
    /// or revocation is superseded (`Error::Pending`) and changes nothing.
    /// Transport failures, timeouts, 5xx, 401 (session) and 409 keep the
    /// published lease; only a typed identity rejection or netmap
    /// withdrawal ends it. A
    /// response not signed by the pinned Server key is rejected
    /// (`SignatureInvalid`, `ServerKeyMismatch`) without changing state or
    /// withdrawing the current, itself verified, lease. An endorsed
    /// successor of the pinned key is pinned once the answer is installed.
    pub async fn finish_sync(
        &mut self,
        reply: SyncReply,
        sink: &mut (dyn PeerFactsSink + Send),
    ) -> Result<SyncOutcome, Error> {
        self.ready()?;
        if !Weak::ptr_eq(&reply.exchange, &Arc::downgrade(&self.netmap.exchange))
            || reply.request != self.sync_request()
        {
            return Err(Error::Pending);
        }
        let response = match reply.result {
            Ok(response) => response,
            Err(error) => {
                // Session/legacy handling is shared with every Node request;
                // only a typed identity rejection withdraws the lease.
                return self.registered(Err(error));
            }
        };
        response
            .check(&reply.request)
            .map_err(|_| Error::Contract)?;
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        // Authority first: nothing of an unsigned or foreign-signed response
        // is decoded into content or changes any state.
        let authority = super::store::authority(
            binding,
            &response.lease,
            &response.signature,
            response.key_rotation.as_ref(),
            self.clock.now()?,
        )?;
        let netmap: Arc<CompiledNetmap> = match response.netmap {
            SyncNetmap::Full { netmap } => Arc::new(*netmap),
            SyncNetmap::Delta { delta } => {
                let base = self.netmap.base.as_ref().ok_or(Error::Contract)?;
                Arc::new(apply_netmap_delta(base, *delta).map_err(|_| Error::Contract)?)
            }
            SyncNetmap::Unchanged => self.netmap.base.clone().ok_or(Error::Contract)?,
        };
        if !super::store::bound(&netmap, &response.lease, binding) {
            return Err(Error::Contract);
        }
        let change = self
            .install_netmap(netmap, response.lease, &authority.signature)
            .await?;
        if let Some((next, signature)) = authority.successor {
            self.pin_server_key(next, response.lease, &signature)
                .await?;
        }
        // Content first: an overlay binds the netmap just installed.
        self.record_associations(response.associations)?;
        let facts = response.peer_facts.is_some();
        if let Some(delta) = response.peer_facts {
            self.netmap.facts = Some(delta.cursor);
            self.record_facts(&delta);
        }
        self.record_traversal(response.traversal_requests);
        self.publish_facts(sink);
        self.persist_facts().await;
        self.persist_floor().await;
        self.netmap.exchange = Arc::new(());
        Ok(SyncOutcome {
            netmap: self.netmap().ok_or(Error::Lifecycle)?,
            change,
            facts,
        })
    }

    /// One complete long poll while holding the driver.
    pub async fn sync(
        &mut self,
        sink: &mut (dyn PeerFactsSink + Send),
    ) -> Result<SyncOutcome, Error> {
        let call = self.begin_sync()?;
        let reply = call.send().await;
        self.finish_sync(reply, sink).await
    }
}

/// The loop ends where control itself must stop; a withdrawn netmap and
/// every retryable failure keep polling. A response that fails verification
/// is retried too: it never replaced the held, itself verified lease.
fn terminal(error: &Error) -> bool {
    match error.recovery() {
        crate::Recovery::Withdrawn { identity } => identity,
        crate::Recovery::Storage => true,
        crate::Recovery::Halt => matches!(error, Error::Endpoint),
        crate::Recovery::Retry { .. } => false,
    }
}

/// Drive the long poll until `stop`. The driver lock is held only to prepare
/// and apply, never during the hold. `errors` holds the class of the last
/// failure while it is retried and is cleared by the next successful sync. Each successful sync publishes its
/// verified netmap; a withdrawal publishes `None`. Session loss re-authenticates;
/// transport failures back off from 1 s doubling to 15 s. Identity rejection,
/// storage that needs reopening and invalid configuration end the loop.
pub async fn sync_loop(
    control: &tokio::sync::Mutex<Control>,
    sink: &mut (dyn PeerFactsSink + Send),
    netmaps: &tokio::sync::watch::Sender<Option<meshway_engine::Netmap>>,
    errors: &tokio::sync::watch::Sender<Option<SyncError>>,
    stop: &CancellationToken,
) -> Result<(), Error> {
    let failed = |error: &Error| {
        errors.send_if_modified(|current| {
            current.replace(error.sync_error()) != Some(error.sync_error())
        });
    };
    let mut backoff = BACKOFF_MIN;
    loop {
        let mut wait = None;
        let mut idle = false;
        let call = {
            let mut driver = tokio::select! { biased;
                _ = stop.cancelled() => return Ok(()),
                driver = control.lock() => driver,
            };
            match driver.begin_sync() {
                Ok(call) => Some(call),
                Err(Error::Lifecycle | Error::Expired) => {
                    // Authenticate without holding the lock across its requests.
                    drop(driver);
                    let authenticated = tokio::select! { biased;
                        _ = stop.cancelled() => return Ok(()),
                        result = crate::shared::authenticate(control) => result,
                    };
                    if let Err(error) = authenticated {
                        failed(&error);
                        let driver = control.lock().await;
                        if driver.netmap().is_none() {
                            netmaps.send_if_modified(|current| current.take().is_some());
                        }
                        if terminal(&error) {
                            return Err(error);
                        }
                        wait = Some(backoff);
                    }
                    None
                }
                Err(error) if terminal(&error) => {
                    failed(&error);
                    if driver.netmap().is_none() {
                        netmaps.send_if_modified(|current| current.take().is_some());
                    }
                    return Err(error);
                }
                Err(error) => {
                    failed(&error);
                    wait = Some(backoff);
                    None
                }
            }
        };
        if let Some(call) = call {
            let reply = tokio::select! { biased;
                _ = stop.cancelled() => return Ok(()),
                reply = call.send() => reply,
            };
            let mut driver = control.lock().await;
            match driver.finish_sync(reply, sink).await {
                Ok(outcome) => {
                    backoff = BACKOFF_MIN;
                    errors.send_if_modified(|current| current.take().is_some());
                    if outcome.change == NetmapChange::Renewed && !outcome.facts {
                        idle = true;
                    }
                    netmaps.send_replace(Some(outcome.netmap));
                    drop(driver);
                    // Content first, then the artifacts it references.
                    super::fetch_artifacts(control, netmaps, stop).await;
                }
                Err(Error::Pending) => {}
                Err(error) => {
                    failed(&error);
                    if driver.netmap().is_none() {
                        netmaps.send_if_modified(|current| current.take().is_some());
                    }
                    if terminal(&error) {
                        return Err(error);
                    }
                    wait = Some(backoff);
                }
            }
        }
        let delay = match wait {
            Some(delay) => {
                backoff = (backoff * 2).min(BACKOFF_MAX);
                Some(delay)
            }
            None => idle.then_some(IDLE_SPACING),
        };
        if let Some(delay) = delay {
            tokio::select! { biased;
                _ = stop.cancelled() => return Ok(()),
                _ = tokio::time::sleep(delay) => {},
            }
        }
    }
}

impl NetmapChange {
    pub fn is_new(&self) -> bool {
        matches!(self, Change::New { .. })
    }
}
