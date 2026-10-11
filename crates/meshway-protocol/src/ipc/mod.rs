//! Local control messages. Responses carry public status; Up may carry a
//! one-use admission secret over the authorized private socket.
#[cfg(unix)]
mod client;
mod codec;
mod secret;
pub use crate::NodeObservationTarget as PingTarget;
use crate::{EnrollmentResponse, InstanceLease, NodeObservationTarget, NodeObservationTargetView};
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
pub use secret::EnrollmentKey;
use serde::Serialize;
use utoipa::ToSchema;

pub const VERSION: u16 = 6;
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
    Ping {
        target: NodeObservationTarget,
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
} }

meshway_model::tagged! { #[derive(Clone, Debug)] pub enum PingEvent {
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
} }
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
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub last_error: Option<Problem> [required = true],
    #[serde(deserialize_with = "meshway_model::wire::required_nullable")]
    pub retry_at: Option<DateTime<Utc>> [required = true],
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
} }
