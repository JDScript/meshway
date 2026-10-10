//! Caller-driven Node control lifecycle over the generated Server client.
//!
//! The host owns scheduling, reconnect backoff and runtime installation. This
//! component owns credentials, pending enrollment, instance binding, the
//! verified netmap and retry identities. It never reports a netmap as applied.
#![forbid(unsafe_code)]

mod authority;
pub mod connectivity;
mod runtime;
pub use runtime::ApplicationTicket;
mod enrollment;
mod identity;
mod instance;
mod netmap;
pub use netmap::{
    FactsRejected, NetmapChange, PeerFactsSink, SYNC_TIMEOUT, SyncCall, SyncFuture, SyncOutcome,
    SyncReply, SyncTransport, sync_loop,
};
mod observations;
mod rotation;
pub use observations::ObservationLease;
mod session;
mod storage;

use meshway_authority::{Clock, Deadline, Started, now};
use meshway_control_client::{Client, ClientError};
use meshway_model::{NetmapLease, NetmapRef, state::Capability};
use meshway_protocol::{
    AcquireInstanceRequest, EnrollmentResponse, ErrorResponse, InstanceLease, NodeSessionResponse,
    StateReport, node::NodeSessionIntent,
};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use storage::Record;

/// Validated receipt rejection details. Debug deliberately omits the remote
/// message; callers may inspect the typed response for code/request correlation.
pub struct ReceiptRejection(ErrorResponse);

impl ReceiptRejection {
    pub fn response(&self) -> &ErrorResponse {
        &self.0
    }
}
impl From<ErrorResponse> for ReceiptRejection {
    fn from(response: ErrorResponse) -> Self {
        Self(response)
    }
}
impl std::fmt::Debug for ReceiptRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReceiptRejection")
            .field("request_id", &self.0.request_id)
            .field("status", &self.0.status)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Server origin or HTTP client configuration")]
    Endpoint,
    #[error("control storage operation failed: {0}")]
    Storage(#[from] meshway_private_storage::Error),
    #[error("private control record is invalid or belongs to a different origin")]
    Record,
    #[error("the previous durable write has an unknown outcome; reopen control storage")]
    ReloadRequired,
    #[error("control operation is not valid in the current lifecycle state")]
    Lifecycle,
    #[error("a different operation is pending; retry or finish it first")]
    Pending,
    #[error("state report scope changed; synchronize and observe again")]
    StateScopeChanged,
    #[error("netmap receipt was rejected; synchronize and apply a new attempt")]
    ReceiptRejected(Box<ReceiptRejection>),
    #[error("secure random generation or signing failed")]
    Cryptography,
    #[error("Server response does not match the requested binding or contract")]
    Contract,
    #[error("netmap signature does not verify with the pinned Server key")]
    SignatureInvalid,
    #[error("netmap was signed by a different Server key than the pinned one")]
    ServerKeyMismatch,
    #[error("the current session or instance has expired")]
    Expired,
    #[error("Node membership was revoked")]
    MembershipRevoked,
    #[error("Server operation failed: {0}")]
    Server(#[from] ClientError<ErrorResponse>),
}

impl From<meshway_authority::Error> for Error {
    fn from(error: meshway_authority::Error) -> Self {
        match error {
            meshway_authority::Error::Expired => Self::Expired,
            meshway_authority::Error::Contract => Self::Contract,
        }
    }
}

/// Public facts only. `netmap` is the published lease while it has not ended.
#[derive(Clone, Debug)]
pub struct Status {
    /// Locally durable intent ID; it may still await successful Server creation.
    pub enrollment_request_id: Option<meshway_model::EnrollmentRequestId>,
    pub binding: Option<EnrollmentResponse>,
    pub authenticated: bool,
    pub instance: Option<InstanceLease>,
    /// Verified netmap content held (the next sync base), with or without authority.
    pub held: Option<NetmapRef>,
    /// Published signed lease of the held netmap, while it has not ended.
    pub netmap: Option<NetmapLease>,
    pub reload_required: bool,
}

/// One control identity and its transient runtime state. Each host must own one
/// mutable driver; no internal reconnect loop or detached network producer exists.
pub struct Control {
    client: Client,
    clock: Arc<Clock>,
    context_validity: Arc<AtomicBool>,
    directory: Arc<meshway_private_storage::Directory>,
    record: Record,
    netmap: netmap::State,
    netmap_write: Option<tokio::task::JoinHandle<Result<(), meshway_private_storage::Error>>>,
    sync_transport: Option<Arc<dyn SyncTransport>>,
    write: Option<tokio::task::JoinHandle<Result<(), meshway_private_storage::Error>>>,
    dirty: bool,
    session: Option<Session>,
    authentication: Option<NodeSessionIntent>,
    challenge_intent: Option<meshway_protocol::NodeChallengeIntent>,
    instance: Option<Instance>,
    acquiring: Option<AcquireInstanceRequest>,
    report: Option<StateReport>,
    report_sequence: u64,
    application_owner: Arc<()>,
    application: Option<ApplicationTicket>,
    connectivity: connectivity::Authority,
    observations: observations::Registry,
}

struct Session {
    response: NodeSessionResponse,
    deadline: Deadline,
    active: bool,
}
struct Instance {
    lease: InstanceLease,
    deadline: Deadline,
    capabilities: BTreeMap<String, Capability>,
}

impl Control {
    /// Synchronous filesystem opening: call from startup or a blocking worker.
    /// Remote origins require HTTPS; plaintext is limited to numeric loopback.
    /// The supplied builder retains the host's socket/resolver protection.
    pub fn open(path: &Path, origin: &str, http: reqwest::ClientBuilder) -> Result<Self, Error> {
        let origin = server_origin(origin)?;
        let client = Client::new(
            &origin,
            // The idle read bound admits the sync long poll; ordinary
            // operations keep the shorter total request timeout below.
            http.connect_timeout(Duration::from_secs(20))
                .read_timeout(SYNC_TIMEOUT),
            storage::LIMIT,
        )
        .map_err(|_| Error::Endpoint)?
        .with_request_timeout(Duration::from_secs(20));
        let directory = Arc::new(meshway_private_storage::Directory::open(
            path,
            "node-control.lock",
        )?);
        let record = Record::load(&directory, &origin)?;
        let mut control = Self {
            client,
            clock: Arc::new(Clock::new()?),
            context_validity: Arc::new(AtomicBool::new(true)),
            directory,
            record,
            netmap: netmap::State::default(),
            netmap_write: None,
            sync_transport: None,
            write: None,
            dirty: false,
            session: None,
            authentication: None,
            challenge_intent: None,
            instance: None,
            acquiring: None,
            report: None,
            report_sequence: 0,
            application_owner: Arc::new(()),
            application: None,
            connectivity: connectivity::Authority::default(),
            observations: observations::Registry::default(),
        };
        control.load_netmap()?;
        Ok(control)
    }

    pub fn status(&self) -> Status {
        self.snapshot().status()
    }

    /// Host projections remain bounded by the same clock and invalidation epoch
    /// even while an HTTP operation holds the mutable control driver.
    pub fn snapshot(&self) -> StatusSnapshot {
        let authenticated = !self.dirty
            && self
                .session
                .as_ref()
                .is_some_and(|s| s.active && s.deadline.valid());
        let status = Status {
            enrollment_request_id: if self.dirty {
                None
            } else {
                self.record.enrollment.as_ref().map(|e| e.intent.id)
            },
            binding: if self.dirty {
                None
            } else {
                self.record.binding.clone()
            },
            authenticated,
            instance: self
                .instance
                .as_ref()
                .filter(|i| authenticated && i.deadline.valid())
                .map(|i| i.lease.clone()),
            held: if self.dirty {
                None
            } else {
                self.netmap_reference()
            },
            netmap: self.netmap_lease(),
            reload_required: self.dirty,
        };
        StatusSnapshot {
            status,
            context_validity: Arc::clone(&self.context_validity),
            session: self.session.as_ref().map(|s| s.deadline.clone()),
            instance: self.instance.as_ref().map(|i| i.deadline.clone()),
            netmap: self.netmap_lease(),
            clock: Arc::clone(&self.clock),
        }
    }

    pub fn pending_hostname(&self) -> Option<&str> {
        self.record
            .enrollment
            .as_ref()
            .map(|e| e.intent.hostname.as_str())
    }
    pub fn can_redeem_enrollment(&self) -> bool {
        self.record
            .enrollment
            .as_ref()
            .is_some_and(|e| e.code.is_some())
    }

    fn ready(&self) -> Result<(), Error> {
        if self.dirty {
            return Err(Error::ReloadRequired);
        }
        Ok(())
    }

    fn token(&self) -> Result<&str, Error> {
        self.ready()?;
        let session = self.session.as_ref().ok_or(Error::Lifecycle)?;
        if !session.active {
            return Err(Error::Lifecycle);
        }
        if !session.deadline.valid() {
            return Err(Error::Expired);
        }
        Ok(session.response.token.expose())
    }

    fn current_instance(&self) -> Result<meshway_protocol::InstanceRequest, Error> {
        self.token()?;
        let instance = self.instance.as_ref().ok_or(Error::Lifecycle)?;
        if !instance.deadline.valid() {
            return Err(Error::Expired);
        }
        Ok(meshway_protocol::InstanceRequest {
            instance_id: instance.lease.instance_id,
            generation: instance.lease.generation,
        })
    }

    /// Drop instance-bound state: State, receipts, coordination and
    /// observations need a new instance. Netmap authority is unaffected.
    fn fence(&mut self) {
        self.invalidate_status_context();
        self.observations = observations::Registry::default();
        self.connectivity = connectivity::Authority::default();
        self.instance = None;
        self.acquiring = None;
        self.report = None;
        self.report_sequence = 0;
        self.application = None;
    }

    fn invalidate_status_context(&mut self) {
        self.context_validity.store(false, Ordering::Release);
        self.context_validity = Arc::new(AtomicBool::new(true));
    }

    /// Only a decoded identity rejection invalidates signed netmap authority.
    /// HTTP/session/resource failures can require fresh HTTP credentials only.
    fn registered<T>(
        &mut self,
        response: Result<T, ClientError<ErrorResponse>>,
    ) -> Result<T, Error> {
        if response.as_ref().is_err_and(identity_rejected) {
            self.block_netmap_cache()?;
        }
        if matches!(
            &response,
            Err(ClientError::Http {
                status: 401 | 403 | 404 | 409,
                ..
            } | ClientError::Network {
                status: Some(401 | 403 | 404 | 409)
            } | ClientError::Decode { .. }
                | ClientError::BodyLimit { .. })
        ) {
            self.fence();
            self.session = None;
            self.challenge_intent = None;
        }
        response.map_err(Error::Server)
    }

    // Only a decoded receipt-domain rejection can retire a receipt without
    // revoking the independently held session/instance or execution guards.
    fn receipt_response<T>(
        &mut self,
        response: Result<T, ClientError<ErrorResponse>>,
    ) -> Result<T, Error> {
        match response {
            Err(ClientError::Http {
                status: 409,
                problem: Some(body),
            }) if body.status == 409
                && matches!(
                    body.problem,
                    meshway_protocol::Problem::ReceiptNetmapSuperseded { .. }
                        | meshway_protocol::Problem::ReceiptPrecondition { .. }
                        | meshway_protocol::Problem::ReceiptContentConflict { .. }
                ) =>
            {
                Err(Error::ReceiptRejected(Box::new(body.into())))
            }
            other => self.registered(other),
        }
    }

    /// Stop local control authority before sending the release. Failure or
    /// cancellation of that request cannot make the local instance live again
    /// or publish the withdrawn lease.
    pub async fn stop(&mut self) -> Result<(), Error> {
        let instance = self
            .instance
            .take()
            .map(|i| meshway_protocol::InstanceRequest {
                instance_id: i.lease.instance_id,
                generation: i.lease.generation,
            });
        self.authentication = None;
        self.challenge_intent = None;
        self.fence();
        self.revoke_netmap();
        if let Some(session) = &mut self.session {
            session.active = false;
        }
        if let (Some(instance), Some(session)) = (instance, self.session.as_ref()) {
            self.client
                .release_node_instance(&instance, session.response.token.expose())
                .await?;
        }
        Ok(())
    }

    /// Await an accepted private write even if the operation's original caller
    /// cancelled. Ordinary dropping also retains the lock in that finite worker.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        let stopped = self.stop().await;
        if let Some(write) = self.write.take() {
            write.await.map_err(|_| Error::ReloadRequired)??;
        }
        if let Some(write) = self.netmap_write.take() {
            write.await.map_err(|_| Error::ReloadRequired)??;
        }
        stopped
    }
}

fn identity_rejected(error: &ClientError<ErrorResponse>) -> bool {
    matches!(error, ClientError::Http { status: 403, problem: Some(body) }
        if body.status == 403 && matches!(body.problem, meshway_protocol::Problem::NodeIdentityRejected { .. }))
}

pub fn server_origin(value: &str) -> Result<String, Error> {
    let url = reqwest::Url::parse(value).map_err(|_| Error::Endpoint)?;
    let loopback = url
        .host_str()
        .and_then(|host| {
            host.trim_start_matches('[')
                .trim_end_matches(']')
                .parse::<std::net::IpAddr>()
                .ok()
        })
        .is_some_and(|ip| ip.is_loopback());
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
    {
        return Err(Error::Endpoint);
    }
    Ok(url.to_string())
}

#[derive(Clone)]
pub struct StatusSnapshot {
    status: Status,
    context_validity: Arc<AtomicBool>,
    session: Option<Deadline>,
    instance: Option<Deadline>,
    netmap: Option<NetmapLease>,
    clock: Arc<Clock>,
}
impl StatusSnapshot {
    pub fn status(&self) -> Status {
        let mut value = self.status.clone();
        value.authenticated &= self.context_validity.load(Ordering::Acquire)
            && self.session.as_ref().is_some_and(Deadline::valid);
        if !value.authenticated || !self.instance.as_ref().is_some_and(Deadline::valid) {
            value.instance = None;
        }
        let now = self.clock.now().ok();
        value.netmap = self
            .netmap
            .filter(|lease| now.is_some_and(|now| lease.valid_at(now)));
        value
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.context_validity.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;

fn random_id() -> Result<uuid::Uuid, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Cryptography)?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}
