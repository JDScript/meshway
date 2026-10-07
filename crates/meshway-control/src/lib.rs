//! Caller-driven Node control lifecycle over the generated Server client.
//!
//! The host owns scheduling, reconnect backoff and runtime installation. This
//! component owns credentials, pending enrollment, instance binding, durable
//! saved targets and retry identities. It never reports a target as applied.
#![forbid(unsafe_code)]

mod authority;
pub mod connectivity;
mod runtime;
use meshway_authority::ExecutionSnapshot;
pub use runtime::ApplicationTicket;
mod demand;
mod dns;
mod enrollment;
mod events;
pub use events::{EventNotice, NodeEvents};
mod execution;
mod identity;
mod observations;
mod rotation;
pub use observations::ObservationLease;
mod session;
mod storage;

use meshway_authority::{Clock, Deadline, Started, now, reference};
use meshway_configuration::execution::CompiledNode;
use meshway_control_client::{Client, ClientError};
use meshway_model::{ConfigurationRef, state::Capability};
use meshway_protocol::{
    AcquireInstanceRequest, EnrollmentResponse, ErrorResponse, InstanceLease, NodeSessionResponse,
    ReceiptRequest, StateReport, node::NodeSessionIntent,
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
    #[error("state report execution scope changed; refresh and observe again")]
    StateScopeChanged,
    #[error("configuration receipt was rejected; refresh and apply a new attempt")]
    ReceiptRejected(Box<ReceiptRejection>),
    #[error("secure random generation or signing failed")]
    Cryptography,
    #[error("Server response does not match the requested binding or contract")]
    Contract,
    #[error("invalid DNS query name")]
    InvalidDnsName,
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

/// Public facts only. Cached configuration is not execution authority.
#[derive(Clone, Debug)]
pub struct Status {
    /// Locally durable intent ID; it may still await successful Server creation.
    pub enrollment_request_id: Option<meshway_model::EnrollmentRequestId>,
    pub binding: Option<EnrollmentResponse>,
    pub authenticated: bool,
    pub instance: Option<InstanceLease>,
    pub cached: Option<ConfigurationRef>,
    pub reload_required: bool,
}

/// One control identity and its transient runtime state. Each host must own one
/// mutable driver; no internal reconnect loop or detached network producer exists.
pub struct Control {
    client: Client,
    clock: Arc<Clock>,
    validity: Arc<AtomicBool>,
    context_validity: Arc<AtomicBool>,
    invalidated: tokio_util::sync::CancellationToken,
    // Private owner/barrier and publication identity, independent of guard
    // invalidations during the lease -> durable-target transaction.
    lineage: Arc<()>,
    publication: Arc<()>,
    predecessor: Option<(Arc<()>, ConfigurationRef)>,
    publication_pending_grant: bool,
    directory: Arc<meshway_private_storage::Directory>,
    record: Record,
    write: Option<tokio::task::JoinHandle<Result<(), meshway_private_storage::Error>>>,
    dirty: bool,
    session: Option<Session>,
    authentication: Option<NodeSessionIntent>,
    challenge_intent: Option<meshway_protocol::NodeChallengeIntent>,
    instance: Option<Instance>,
    acquiring: Option<AcquireInstanceRequest>,
    receipt: Option<SavedReceipt>,
    report: Option<StateReport>,
    report_sequence: u64,
    application_owner: Arc<()>,
    application: Option<ApplicationTicket>,
    connectivity: connectivity::Authority,
    observations: observations::Registry,
    events_cancel: tokio_util::sync::CancellationToken,
}

struct Session {
    response: NodeSessionResponse,
    deadline: Deadline,
    active: bool,
}
struct Instance {
    lease: InstanceLease,
    deadline: Deadline,
    execution_deadline: Option<Deadline>,
    capabilities: BTreeMap<String, Capability>,
}

struct SavedReceipt {
    request: ReceiptRequest,
    acknowledged: bool,
}

impl Control {
    /// Synchronous filesystem opening: call from startup or a blocking worker.
    /// Remote origins require HTTPS; plaintext is limited to numeric loopback.
    /// The supplied builder retains the host's socket/resolver protection.
    pub fn open(path: &Path, origin: &str, http: reqwest::ClientBuilder) -> Result<Self, Error> {
        let origin = server_origin(origin)?;
        let client = Client::new(
            &origin,
            http.connect_timeout(Duration::from_secs(20))
                .read_timeout(Duration::from_secs(30)),
            storage::LIMIT,
        )
        .map_err(|_| Error::Endpoint)?
        .with_request_timeout(Duration::from_secs(20));
        let directory = Arc::new(meshway_private_storage::Directory::open(
            path,
            "node-control.lock",
        )?);
        let record = Record::load(&directory, &origin)?;
        Ok(Self {
            client,
            clock: Arc::new(Clock::new()?),
            validity: Arc::new(AtomicBool::new(true)),
            context_validity: Arc::new(AtomicBool::new(true)),
            invalidated: tokio_util::sync::CancellationToken::new(),
            lineage: Arc::new(()),
            publication: Arc::new(()),
            predecessor: None,
            publication_pending_grant: false,
            directory,
            record,
            write: None,
            dirty: false,
            session: None,
            authentication: None,
            challenge_intent: None,
            instance: None,
            acquiring: None,
            receipt: None,
            report: None,
            report_sequence: 0,
            application_owner: Arc::new(()),
            application: None,
            connectivity: connectivity::Authority::default(),
            observations: observations::Registry::default(),
            events_cancel: tokio_util::sync::CancellationToken::new(),
        })
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
                .map(|i| {
                    let mut lease = i.lease.clone();
                    if !i.execution_deadline.as_ref().is_some_and(|d| d.valid()) {
                        lease.execution_expires_at_ms = None;
                    }
                    lease
                }),
            cached: self.cached_target().map(reference),
            reload_required: self.dirty,
        };
        StatusSnapshot {
            status,
            validity: Arc::clone(&self.validity),
            context_validity: Arc::clone(&self.context_validity),
            session: self.session.as_ref().map(|s| s.deadline.clone()),
            instance: self.instance.as_ref().map(|i| i.deadline.clone()),
            execution: self
                .instance
                .as_ref()
                .and_then(|i| i.execution_deadline.clone()),
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

    fn invalidate_snapshots(&mut self) {
        self.lineage = Arc::new(());
        self.predecessor = None;
        self.publication_pending_grant = false;
        self.invalidate_guards();
    }

    fn invalidate_guards(&mut self) {
        self.validity.store(false, Ordering::Release);
        self.invalidated.cancel();
        self.validity = Arc::new(AtomicBool::new(true));
        self.invalidated = tokio_util::sync::CancellationToken::new();
    }

    /// Private cached configuration, not an applied installation or a lease.
    pub fn cached_target(&self) -> Option<&CompiledNode> {
        if self.dirty {
            None
        } else {
            self.record.saved.as_ref()
        }
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

    fn fence(&mut self) {
        self.invalidate_status_context();
        self.reset_events();
        self.observations = observations::Registry::default();
        self.invalidate_snapshots();
        self.instance = None;
        self.acquiring = None;
        self.receipt = None;
        self.report = None;
        self.report_sequence = 0;
        self.application = None;
    }

    fn invalidate_status_context(&mut self) {
        self.context_validity.store(false, Ordering::Release);
        self.context_validity = Arc::new(AtomicBool::new(true));
    }

    fn registered<T>(
        &mut self,
        response: Result<T, ClientError<ErrorResponse>>,
    ) -> Result<T, Error> {
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
                    meshway_protocol::Problem::ReceiptPublicationSuperseded { .. }
                        | meshway_protocol::Problem::ReceiptLeaseExpired { .. }
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
    /// cancellation of that request cannot make the local instance live again.
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
        stopped
    }
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
    validity: Arc<AtomicBool>,
    context_validity: Arc<AtomicBool>,
    session: Option<Deadline>,
    instance: Option<Deadline>,
    execution: Option<Deadline>,
}
impl StatusSnapshot {
    pub fn status(&self) -> Status {
        let mut value = self.status.clone();
        value.authenticated &= self.context_validity.load(Ordering::Acquire)
            && self.session.as_ref().is_some_and(Deadline::valid);
        if !value.authenticated || !self.instance.as_ref().is_some_and(Deadline::valid) {
            value.instance = None;
        }
        if let Some(instance) = &mut value.instance
            && (!self.validity.load(Ordering::Acquire)
                || !self.execution.as_ref().is_some_and(Deadline::valid))
        {
            instance.execution_expires_at_ms = None;
        }
        value
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.context_validity.store(false, Ordering::Release);
        self.events_cancel.cancel();
        self.validity.store(false, Ordering::Release);
        self.invalidated.cancel();
    }
}

#[cfg(test)]
mod tests;

fn random_id() -> Result<uuid::Uuid, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::Cryptography)?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}
