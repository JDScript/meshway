//! Local control messages. Responses carry public status; Up may carry a
//! one-use admission secret over the authorized private socket.
#[cfg(unix)]
mod client;
mod codec;
mod ping;
mod secret;
mod view;
use crate::{EnrollmentResponse, InstanceLease, NodeObservationTargetView};
use chrono::{DateTime, Utc};
#[cfg(unix)]
pub use client::{Client, ClientError, PingStream};
pub use codec::{CodecError, read, write};
use meshway_model::{
    EnrollmentRequestId, NetmapRef, ObservationId, ObservationSubscriptionId, RequestId, Sequence,
    Version, observations::ObservationSample,
};
pub use meshway_model::{
    observations::{EndpointKind, LinkAcquisition},
    state::ProbeOutcome,
};
pub use ping::{
    PingAction, PingAssociation, PingDecision, PingEcho, PingEchoOutcome, PingIcmpError,
    PingResolution, PingResolver, PingRule, PingTarget,
};
pub use secret::EnrollmentKey;
use serde::Serialize;
use utoipa::ToSchema;
pub use view::{
    EndpointMapping, IdlePath, IdleRoute, NetcheckObserver, NetcheckReport, NetcheckServer,
    NetcheckSizeProbe, NetcheckSocket, NetcheckUnavailable, NodeSelf, PeerMtu, PeerRtt, PeerState,
    PeerStatus, PortMapping, SocketStatus, StunMapping,
};

pub const VERSION: u16 = 12;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DEFAULT_SOCKET: &str = "/var/lib/meshway/control.sock";

meshway_model::object! { #[derive(Clone, Debug)] pub struct Request {
    pub version: u16,
    pub request_id: RequestId,
    pub command: Command,
} }
meshway_model::tagged! {
#[derive(Clone, Debug)]
pub enum Command {
    Status,
    Up {
        #[serde(rename = "expectedRevision")]
        expected_revision: Version,
        #[serde(default, skip_serializing_if = "Option::is_none", rename = "enrollmentKey")]
        enrollment_key: Option<EnrollmentKey>,
        /// Enroll the held identity again (same Node and keys, Console approval) to pin the Server's current signing key.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reenroll: bool,
    },
    RotateKey {
        #[serde(rename = "expectedRevision")]
        expected_revision: Version,
    },
    Down {
        #[serde(rename = "expectedRevision")]
        expected_revision: Version,
    },
    /// Ask the Server to delete this Node, then forget its identity and go down.
    Logout {
        #[serde(rename = "expectedRevision")]
        expected_revision: Version,
    },
    /// One bounded local network diagnostic.
    Netcheck,
    /// Ask the Server to select this Node's routing profile; answers its committed selection.
    SelectProfile {
        profile: String,
    },
    /// Ask the Server to clear this Node's routing selection so that the assignment's default applies; answers as SelectProfile.
    ClearProfile,
    Ping {
        target: PingTarget,
        #[serde(deserialize_with = "ping_count")]
        count: u16 [minimum = 1, maximum = 1000],
    },
}
}

fn ping_count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let count = <u16 as serde::Deserialize>::deserialize(d)?;
    if !(1..=1000).contains(&count) {
        return Err(serde::de::Error::custom(
            "ping count must be between 1 and 1000",
        ));
    }
    Ok(count)
}

meshway_model::object! { #[derive(Clone, Debug)] pub struct Response {
    pub version: u16,
    pub request_id: RequestId,
    pub result: ResultBody,
} }
meshway_model::tagged! { #[derive(Clone, Debug)] pub enum ResultBody {
    Status { status: Box<crate::ipc::Status> },
    Error { code: ErrorCode },
    Ping { sequence: Sequence, event: PingEvent },
    Netcheck { report: Box<crate::ipc::NetcheckReport> },
    /// The Server's committed selection; the Node's next netmap carries it.
    Selection { selection: Box<meshway_configuration::selection::RoutingSelectionState> },
} }

meshway_model::tagged! { #[derive(Clone, Debug)] pub enum PingEvent {
    /// First event: this Node's local decision for the target, before any probe.
    Decision { decision: PingDecision },
    /// A Mesh Node target: its measurement subscription.
    Accepted {
        #[serde(rename = "subscriptionId")]
        subscription_id: ObservationSubscriptionId,
        #[serde(rename = "observationId")]
        observation_id: ObservationId,
        target: NodeObservationTargetView,
    },
    Preparing {
        #[serde(rename = "elapsedMs")]
        elapsed_ms: Sequence,
    },
    Sample {
        #[serde(rename = "observationId")]
        observation_id: ObservationId,
        sample: ObservationSample,
    },
    /// One echo request toward a target outside the Mesh.
    Echo { echo: PingEcho },
    Completed { sent: u32, success: u32, timeout: u32, failed: u32 },
    Failed { code: PingError },
} }

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PingError {
    NotRunning,
    Unsupported,
    PermissionDenied,
    TargetNotFound,
    Timeout,
    Busy,
    Stopped,
    Protocol,
    Failed,
}
}

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema, thiserror::Error)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    #[error("Daemon protocol version differs.")]
    Version,
    #[error("Node intent changed; inspect status and retry the command.")]
    Conflict,
    #[error("Node intent changed; inspect status and retry the command.")]
    RequestConflict,
    #[error("Daemon command queue is busy.")]
    Busy,
    #[error("Daemon is stopping.")]
    Stopping,
    #[error("Daemon could not commit its private state.")]
    Storage,
    #[error("Node identity is not ready for this operation.")]
    NotReady,
    #[error("An earlier identity operation is awaiting recovery.")]
    Pending,
    #[error("Server could not be reached; inspect identity status before retrying.")]
    Network,
    #[error("Server temporarily rejected the identity operation.")]
    ServerBusy,
    #[error("Server rejected the identity credentials.")]
    Credentials,
    #[error("The identity response did not match the expected binding.")]
    Contract,
    #[error("Server rejected the request; the daemon log names its reason.")]
    Rejected,
    #[error("The Node is leaving its network and retries until the Server confirms; `meshway up` with a new enrollment key replaces the pending leave.")]
    Leaving,
    #[error("The Node's assignment does not allow this routing profile.")]
    ProfileNotAllowed,
    #[error("An administrator locked this Node's routing profile.")]
    ProfileLocked,
    #[error("The routing selection changed elsewhere; retry once the Node has the new netmap.")]
    SelectionConflict,
}
}
meshway_model::string_enum! {
/// The Node's `sync` long poll with the Server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    /// The last sync succeeded and the next one is held by the Server.
    Connected,
    /// The last sync failed; it is being retried with backoff.
    Reconnecting,
    /// No sync runs (down, not enrolled, or stopped).
    #[default]
    Disconnected,
}
}

meshway_model::string_enum! {
/// Why the last `sync` failed while it is retried; never a withdrawal of the
/// held lease by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SyncError {
    /// The Server could not be reached or the request timed out.
    Network,
    /// The Server answered busy or unavailable (429/5xx).
    ServerBusy,
    /// The Node session or instance must be acquired again.
    Credentials,
    /// The answer's signature does not verify with the pinned Server key.
    SignatureInvalid,
    /// The answer is signed by another Server key than the pinned one (for example a missed rotation window).
    ServerKeyMismatch,
    /// The Server withdrew this Node's netmap.
    Withdrawn,
    /// The Server rejected this Node identity.
    IdentityRejected,
    /// The answer does not match the request or the protocol.
    Contract,
    /// Private storage of the answer failed.
    Storage,
}
}

meshway_model::object! {
/// The installed netmap and its lease; the data plane's only deadline.
#[derive(Clone, Debug)] pub struct NetmapStatus {
    pub netmap: NetmapRef,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Rule-set artifacts the netmap references that this Node does not
    /// hold yet (`sha256:<hex>`); rules naming them are unavailable.
    pub missing_artifacts: Vec<String>,
    /// Last fetch or storage failure while an artifact is missing; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub artifact_problem: Option<ArtifactProblem> [required = true],
    /// Egress capture this Node computed from the netmap, its artifacts and
    /// its live DNS associations, and installed: interval elements,
    /// capture-table routes, or why only the full boundary is installed;
    /// null for a disabled Node.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub capture: Option<meshway_configuration::evaluation::CaptureSummary> [required = true],
    /// This Node's routing profile selection as the netmap decides it;
    /// null when it cannot be computed from the netmap.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub routing: Option<meshway_configuration::selection::RoutingSelectionState> [required = true],
} }
meshway_model::string_enum! {
/// Why a referenced rule-set artifact is not held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactProblem {
    /// The Server did not answer with the artifact.
    Download,
    /// The bytes differ from the netmap reference or are not a valid artifact.
    Integrity,
    /// The private artifact cache could not be written or read.
    Storage,
    /// The referenced artifacts together exceed 64 MiB.
    Budget,
}
}
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Down,
    Starting,
    AwaitingApproval,
    Synchronizing,
    ControlReady,
    Running,
    Retrying,
    Blocked,
    Stopping,
    /// A durable leave intent awaits the Server's confirmation; it is retried
    /// and nothing rejoins meanwhile.
    Leaving,
}
}
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    Network,
    ServerBusy,
    Credentials,
    EnrollmentExpired,
    EnrollmentWithdrawn,
    EnrollmentCancelled,
    Storage,
    Contract,
    Configuration,
    Clock,
    Worker,
    UnsupportedMode,
    Underlay,
    Tun,
    Dns,
    Transport,
    Apply,
}
}
meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum DataPlane {
    ControlOnly,
    Stopped,
    Preparing,
    Ready,
    Suspended,
    Failed,
}
}

meshway_model::string_enum! {
/// Local device attachment choice, independent of cloud business policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum NetworkMode {
    #[default]
    Tun,
    ControlOnly,
}
}

impl std::str::FromStr for NetworkMode {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "tun" => Ok(Self::Tun),
            "control-only" => Ok(Self::ControlOnly),
            _ => Err("Expected tun or control-only; userspace networking is not implemented."),
        }
    }
}

meshway_model::object! { #[derive(Clone, Debug)] pub struct Status {
    pub process_id: u32,
    pub revision: Version,
    pub desired_up: bool,
    pub phase: Phase,
    pub server: String,
    pub observed_at: DateTime<Utc>,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub enrollment_request_id: Option<EnrollmentRequestId> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub enrollment_url: Option<String> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub identity: Option<EnrollmentResponse> [required = true],
    pub authenticated: bool,
    pub sync: SyncState,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub last_sync_at: Option<DateTime<Utc>> [required = true],
    /// Failure class of the last sync while the long poll retries; null after a successful sync or when no sync runs.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub sync_error: Option<SyncError> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub instance: Option<InstanceLease> [required = true],
    /// Current netmap authority; null when no lease is live.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub netmap: Option<NetmapStatus> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub saved: Option<NetmapRef> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub applied: Option<NetmapRef> [required = true],
    pub data_plane: DataPlane,
    pub network_mode: NetworkMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<ExitState>,
    /// This Node in the installed netmap; null without one.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub node: Option<NodeSelf> [required = true],
    /// Visible peers of the installed netmap; empty without one.
    pub peers: Vec<PeerStatus>,
    /// Native UDP sockets of the installed netmap's listeners and dialers; empty without one.
    pub sockets: Vec<SocketStatus>,
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub last_error: Option<Problem> [required = true],
    /// The concrete cause of `lastError` when the failing component names one,
    /// e.g. `listener lab-tcp cannot bind 0.0.0.0:443: Address in use (os error 98)`; null otherwise.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub last_error_detail: Option<String> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub retry_at: Option<DateTime<Utc>> [required = true],
    /// The packet plane's own failure while `lastError` is another
    /// component's (Control): its class and cause, which `lastError` does
    /// not carry then; omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_error: Option<NetworkError>,
    /// When the Underlay last classified an external network change of
    /// this host (a path change or a refresh); omitted before the first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_network_change: Option<DateTime<Utc>>,
    /// A kernel change of the egress capture or exit owner whose attempt
    /// failed and that the owner retries after reading the kernel back (at
    /// most 30 s from the first failure); omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pending: Option<KernelChange>,
} }

meshway_model::string_enum! {
/// The Underlay kernel owner of a retried change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum KernelChangeOwner {
    /// The egress capture owner (`pause`, `stage`, `switch`).
    EgressCapture,
    /// The exit role owner: a projection's retirement after the named
    /// request (`retire`, `renew`, `install`, `confirm`), or `maintain`.
    Exit,
}
}

meshway_model::object! {
/// A failure of the packet plane, with its cause when it names one.
#[derive(Clone, Debug)] pub struct NetworkError {
    pub problem: Problem,
    /// As `lastErrorDetail` for this failure; null when it names none.
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub detail: Option<String> [required = true],
} }

meshway_model::object! {
/// A failed kernel change its owner retries.
#[derive(Clone, Debug)] pub struct KernelChange {
    pub owner: KernelChangeOwner,
    pub operation: String,
    /// The last attempt's error.
    pub reason: String,
    /// Attempts so far, the first one included.
    pub attempts: u32,
    /// When the next attempt starts (1 s after the first failure, doubling).
    pub retry_at: DateTime<Utc>,
} }

meshway_model::object! {
/// Target-bound facts from the installed exit adapter. Missing measurements
/// remain absent, including before the first probe and after authority loss.
#[derive(Clone, Debug)] pub struct ExitState {
    pub capability: Option<meshway_model::exit::ExitCapability>,
    pub installed_role: Option<meshway_model::Digest>,
    pub active_flows: Option<u32>,
    /// Why this exit resolver answers its sources SERVFAIL; null while it serves or serves no source.
    pub dns_unavailable: Option<meshway_model::exit::ExitDnsUnavailableReason>,
    /// A stopped exit role whose cleanup failed: its resources are kept and
    /// the cleanup is retried; the role installs again once it completed.
    /// Omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_pending: Option<ExitCleanup>,
} }

meshway_model::object! {
/// The retried cleanup of a stopped exit role.
#[derive(Clone, Debug)] pub struct ExitCleanup {
    /// The last attempt's error.
    pub reason: String,
    /// Attempts so far, the first one included.
    pub attempts: u32,
    /// When the next attempt starts (1 s after the first failure, doubling to 60 s).
    pub retry_at: DateTime<Utc>,
} }
