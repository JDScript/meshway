//! Local control messages. These carry public status, never Node credentials.
#![forbid(unsafe_code)]
mod codec;
use chrono::{DateTime, Utc};
pub use codec::{CodecError, read, write};
use meshway_model::{ConfigurationRef, EnrollmentRequestId, RequestId, Version};
use meshway_protocol::{EnrollmentResponse, InstanceLease};
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub const DEFAULT_SOCKET: &str = "/var/lib/meshway/control.sock";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub version: u16,
    pub request_id: RequestId,
    pub command: Command,
}
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
}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Response {
    pub version: u16,
    pub request_id: RequestId,
    pub result: ResultBody,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum ResultBody {
    Status { status: Box<Status> },
    Error { code: ErrorCode },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Version,
    Conflict,
    RequestConflict,
    Busy,
    Stopping,
    Storage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Down,
    Starting,
    AwaitingApproval,
    Synchronizing,
    ControlReady,
    Retrying,
    Blocked,
    Stopping,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataPlane {
    NotImplemented,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Status {
    pub process_id: u32,
    pub revision: Version,
    pub desired_up: bool,
    pub phase: Phase,
    pub server: String,
    pub observed_at: DateTime<Utc>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub enrollment_request_id: Option<EnrollmentRequestId>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub enrollment_url: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub identity: Option<EnrollmentResponse>,
    pub authenticated: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    pub instance: Option<InstanceLease>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub saved: Option<ConfigurationRef>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub applied: Option<ConfigurationRef>,
    pub data_plane: DataPlane,
    #[serde(deserialize_with = "Option::deserialize")]
    pub last_error: Option<Problem>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub retry_at: Option<DateTime<Utc>>,
}
