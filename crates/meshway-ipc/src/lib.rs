//! Local control messages. These carry public status, never Node credentials.
#![forbid(unsafe_code)]
#[cfg(unix)]
mod client;
mod codec;
use chrono::{DateTime, Utc};
#[cfg(unix)]
pub use client::{Client, ClientError, PingStream};
pub use codec::{CodecError, read, write};
use meshway_model::{
    ConfigurationRef, EnrollmentRequestId, ObservationId, ObservationSubscriptionId, RequestId,
    Sequence, Version, observations::ObservationSample,
};
pub use meshway_model::{
    observations::{EndpointKind, LinkAcquisition},
    state::ProbeOutcome,
};
pub use meshway_protocol::NodeObservationTarget as PingTarget;
use meshway_protocol::{
    EnrollmentResponse, InstanceLease, NodeObservationTarget, NodeObservationTargetView,
};
use serde::Serialize;
use utoipa::ToSchema;

pub const VERSION: u16 = 3;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DEFAULT_SOCKET: &str = "/var/lib/meshway/control.sock";

meshway_serde::object! { #[derive(Clone, Debug)] pub struct Request {
    pub version: u16,
    pub request_id: RequestId,
    pub command: Command,
} }
meshway_serde::tagged! {
#[derive(Clone, Debug)]
pub enum Command {
    Status,
    Up {
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

meshway_serde::object! { #[derive(Clone, Debug)] pub struct Response {
    pub version: u16,
    pub request_id: RequestId,
    pub result: ResultBody,
} }
meshway_serde::tagged! { #[derive(Clone, Debug)] pub enum ResultBody {
    Status { status: Box<crate::Status> },
    Error { code: ErrorCode },
    Ping { sequence: Sequence, event: PingEvent },
} }

meshway_serde::tagged! { #[derive(Clone, Debug)] pub enum PingEvent {
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

meshway_serde::string_enum! {
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

meshway_serde::string_enum! {
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
}
}
meshway_serde::string_enum! {
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
meshway_serde::string_enum! {
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
meshway_serde::string_enum! {
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

meshway_serde::string_enum! {
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

meshway_serde::object! { #[derive(Clone, Debug)] pub struct Status {
    pub process_id: u32,
    pub revision: Version,
    pub desired_up: bool,
    pub phase: Phase,
    pub server: String,
    pub observed_at: DateTime<Utc>,
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub enrollment_request_id: Option<EnrollmentRequestId> [required = true],
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub enrollment_url: Option<String> [required = true],
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub identity: Option<EnrollmentResponse> [required = true],
    pub authenticated: bool,
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub instance: Option<InstanceLease> [required = true],
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub saved: Option<ConfigurationRef> [required = true],
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub applied: Option<ConfigurationRef> [required = true],
    pub data_plane: DataPlane,
    pub network_mode: NetworkMode,
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub last_error: Option<Problem> [required = true],
    #[serde(deserialize_with = "meshway_serde::required_nullable")]
    pub retry_at: Option<DateTime<Utc>> [required = true],
} }
