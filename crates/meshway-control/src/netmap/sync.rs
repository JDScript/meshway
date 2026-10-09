//! `POST /agent-api/v1/sync`: the single Node long poll.
//!
//! A call is prepared under the Control driver ([`Control::begin_sync`]), sent
//! without holding it ([`SyncCall::send`]) and applied under it again
//! ([`Control::finish_sync`]). The netmap part is checked against the request,
//! verified (full), reconstructed and digest-checked (delta) or matched to the
//! held content (unchanged), bound to this identity, saved and issued. Peer
//! facts are passed through to a [`PeerFactsSink`] afterwards; they never
//! reach a snapshot.
use super::NetmapChange as Change;
use crate::{Control, Error, Started};
use meshway_authority::{Deadline, ExecutionSnapshot};
use meshway_configuration::netmap::{CompiledNetmap, apply_netmap_delta};
use meshway_control_client::ClientError;
use meshway_model::NetmapRef;
use meshway_protocol::{ErrorResponse, PeerFactsDelta, SyncNetmap, SyncRequest, SyncResponse};
use std::{
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
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Spacing after an immediate answer that changed nothing, so a Server that
/// does not hold cannot drive a tight loop.
const IDLE_SPACING: Duration = Duration::from_secs(1);

pub type SyncFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SyncResponse, ClientError<ErrorResponse>>> + Send + 'a>>;

/// One sync HTTP exchange with the Node session token. The production
/// implementation is the generated client operation for the sync route; its
/// client must allow at least [`SYNC_TIMEOUT`] for the total request and read.
pub trait SyncTransport: Send + Sync {
    fn sync<'a>(&'a self, request: &'a SyncRequest, token: &'a str) -> SyncFuture<'a>;
}

/// The facts consumer refused this delta (e.g. its bounded queue is full).
/// Control then requests complete facts on the next sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FactsRejected;

/// Consumer of peer facts (the connectivity Manager in the daemon). Facts are
/// selection input only; the netmap named here is already installed.
pub trait PeerFactsSink {
    fn apply(&mut self, netmap: NetmapRef, facts: PeerFactsDelta) -> Result<(), FactsRejected>;
}

/// What a successful sync did to the netmap authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetmapChange {
    /// New content (first installation or a larger serial). Old guards were
    /// cancelled; consumers compare contents (`netmap_continuation_from`).
    New { previous: Option<NetmapRef> },
    /// Same content, lease renewed in place: every live guard continues and
    /// the new snapshot is `same_netmap` with the previous one.
    Renewed,
    /// Same content, but no live authority was installed (expired, revoked or
    /// restarted with an expired cache): a fresh issue, not `same_netmap`.
    Reissued,
}

pub struct SyncOutcome {
    pub snapshot: ExecutionSnapshot,
    pub change: NetmapChange,
    /// A facts delta was accepted by the sink.
    pub facts: bool,
}

/// A prepared request, independent of the Control borrow.
pub struct SyncCall {
    transport: Arc<dyn SyncTransport>,
    token: Zeroizing<String>,
    request: SyncRequest,
    started: Started,
    epoch: Weak<()>,
}

/// The transport outcome of one call, to be applied by the same Control.
pub struct SyncReply {
    request: SyncRequest,
    started: Started,
    epoch: Weak<()>,
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
            started: self.started,
            epoch: self.epoch,
            result,
        }
    }
}

impl Control {
    pub fn set_sync_transport(&mut self, transport: Arc<dyn SyncTransport>) {
        self.sync_transport = Some(transport);
    }

    /// Prepare one long poll naming the held netmap and facts cursor. Needs a
    /// live session only; no instance is required.
    pub fn begin_sync(&self) -> Result<SyncCall, Error> {
        let token = Zeroizing::new(self.token()?.to_owned());
        Ok(SyncCall {
            transport: self.sync_transport.clone().ok_or(Error::Endpoint)?,
            token,
            request: self.sync_request(),
            started: Started::now(&self.clock)?,
            epoch: Arc::downgrade(&self.netmap.epoch),
        })
    }

    fn sync_request(&self) -> SyncRequest {
        SyncRequest {
            netmap: self.netmap_reference(),
            state: self.netmap.facts,
        }
    }

    /// Apply one reply. A reply prepared before any later installation, stop
    /// or revocation is superseded (`Error::Pending`) and changes nothing.
    /// Transport failures, timeouts, 5xx, 401 (session) and 409 keep the
    /// netmap authority; 403/404 reject this identity and revoke it.
    pub async fn finish_sync(
        &mut self,
        reply: SyncReply,
        sink: &mut dyn PeerFactsSink,
    ) -> Result<SyncOutcome, Error> {
        self.ready()?;
        if !Weak::ptr_eq(&reply.epoch, &Arc::downgrade(&self.netmap.epoch))
            || reply.request != self.sync_request()
        {
            return Err(Error::Pending);
        }
        let response = match reply.result {
            Ok(response) => response,
            Err(error) => {
                let rejected = matches!(
                    &error,
                    ClientError::Http {
                        status: 403 | 404,
                        ..
                    } | ClientError::Network {
                        status: Some(403 | 404)
                    }
                );
                if rejected {
                    self.revoke_netmap();
                }
                // Session/legacy handling is shared with every Node request;
                // it never touches the netmap authority.
                return self.registered(Err(error));
            }
        };
        response
            .check(&reply.request)
            .map_err(|_| Error::Contract)?;
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
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
        let deadline = Deadline::from_response(reply.started, response.lease.expires_at_ms)?;
        let reference = netmap.reference();
        let change = self
            .install_netmap(netmap, response.lease, deadline)
            .await?;
        let mut facts = false;
        if let Some(delta) = response.peer_facts {
            let cursor = delta.cursor;
            if sink.apply(reference, delta).is_ok() {
                self.netmap.facts = Some(cursor);
                facts = true;
            } else {
                self.netmap.facts = None;
            }
        }
        self.netmap.epoch = Arc::new(());
        Ok(SyncOutcome {
            snapshot: self.netmap_snapshot()?,
            change,
            facts,
        })
    }

    /// One complete long poll while holding the driver.
    pub async fn sync(&mut self, sink: &mut dyn PeerFactsSink) -> Result<SyncOutcome, Error> {
        let call = self.begin_sync()?;
        let reply = call.send().await;
        self.finish_sync(reply, sink).await
    }
}

fn terminal(error: &Error) -> bool {
    matches!(
        error,
        Error::ReloadRequired | Error::Record | Error::Endpoint | Error::MembershipRevoked
    ) || matches!(
        error,
        Error::Server(
            ClientError::Http {
                status: 403 | 404,
                ..
            } | ClientError::Network {
                status: Some(403 | 404)
            }
        )
    )
}

/// Drive the long poll until `stop`. The driver lock is held only to prepare
/// and apply, never during the hold. Each successful sync publishes its
/// snapshot; a revocation publishes `None`. Session loss re-authenticates;
/// transport failures back off from 1 s doubling to 30 s. Identity rejection,
/// storage that needs reopening and invalid configuration end the loop.
pub async fn sync_loop(
    control: &tokio::sync::Mutex<Control>,
    sink: &mut dyn PeerFactsSink,
    snapshots: &tokio::sync::watch::Sender<Option<ExecutionSnapshot>>,
    stop: &CancellationToken,
) -> Result<(), Error> {
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
                    if let Err(error) = driver.authenticate().await {
                        if terminal(&error) {
                            return Err(error);
                        }
                        wait = Some(backoff);
                    }
                    None
                }
                Err(error) if terminal(&error) => return Err(error),
                Err(_) => {
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
                    if outcome.change == NetmapChange::Renewed && !outcome.facts {
                        idle = true;
                    }
                    snapshots.send_replace(Some(outcome.snapshot));
                }
                Err(Error::Pending) => {}
                Err(error) => {
                    if driver.netmap_snapshot().is_err() {
                        snapshots.send_if_modified(|current| current.take().is_some());
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
