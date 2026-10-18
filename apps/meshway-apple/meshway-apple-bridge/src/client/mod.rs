//! App-side IPC adapter. No Node, Control, network, storage or logging startup.
//! Decode the entire closed IPC contract before projecting presentation records.
mod view;
pub use view::*;
mod samples;
#[cfg(test)]
mod tests;

use meshway_model::{RequestId, Version};
use meshway_protocol::ipc;
use std::sync::{Arc, Mutex};

/// Pure startup-origin validation shared with Control; no client or Node opens.
#[uniffi::export]
pub fn client_server_origin(value: String) -> Result<String, ClientError> {
    meshway_control::server_origin(&value).map_err(|_| ClientError::Input)
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ClientError {
    #[error("Invalid command input")]
    Input,
    #[error("Invalid, incomplete or oversized IPC frame")]
    Frame,
    #[error("IPC version, request, result or stream sequence differs")]
    Contract,
    #[error("Request already completed")]
    Completed,
}

#[derive(uniffi::Enum)]
pub enum ClientCommand {
    Status,
    Up {
        revision: u64,
        enrollment_key: Option<String>,
        reenroll: bool,
    },
    Down {
        revision: u64,
    },
    RotateKey {
        revision: u64,
    },
    Logout {
        revision: u64,
    },
    Netcheck,
    SelectProfile {
        profile: String,
    },
    ClearProfile,
    Ping {
        target: String,
        count: u16,
    },
}
impl TryFrom<ClientCommand> for ipc::Command {
    type Error = ClientError;
    fn try_from(value: ClientCommand) -> Result<Self, Self::Error> {
        let revision = |v| Version::new(v).map_err(|_| ClientError::Input);
        Ok(match value {
            ClientCommand::Status => Self::Status,
            ClientCommand::Up {
                revision: v,
                enrollment_key,
                reenroll,
            } => {
                if reenroll && enrollment_key.is_some() {
                    return Err(ClientError::Input);
                }
                Self::Up {
                    expected_revision: revision(v)?,
                    enrollment_key: enrollment_key
                        .map(ipc::EnrollmentKey::new)
                        .transpose()
                        .map_err(|_| ClientError::Input)?,
                    reenroll,
                }
            }
            ClientCommand::Down { revision: v } => Self::Down {
                expected_revision: revision(v)?,
            },
            ClientCommand::RotateKey { revision: v } => Self::RotateKey {
                expected_revision: revision(v)?,
            },
            ClientCommand::Logout { revision: v } => Self::Logout {
                expected_revision: revision(v)?,
            },
            ClientCommand::Netcheck => Self::Netcheck,
            ClientCommand::SelectProfile { profile } => Self::SelectProfile { profile },
            ClientCommand::ClearProfile => Self::ClearProfile,
            ClientCommand::Ping { target, count } => {
                if target.is_empty() || target.len() > 253 || !(1..=1000).contains(&count) {
                    return Err(ClientError::Input);
                }
                let target = if target.parse::<std::net::IpAddr>().is_ok() {
                    ipc::PingTarget::Address { address: target }
                } else {
                    ipc::PingTarget::Name { name: target }
                };
                Self::Ping { target, count }
            }
        })
    }
}

#[derive(uniffi::Enum)]
// UniFFI record payloads cross the ABI by value; Box<Record> is not an FFI type.
// Only one bounded response is decoded per request, outside the packet path.
#[allow(clippy::large_enum_variant)]
pub enum ClientResult {
    Status {
        status: ClientStatus,
    },
    Error {
        code: ClientErrorCode,
        message: String,
    },
    Selection {
        selection: ClientSelection,
    },
    Netcheck {
        report: ClientNetcheck,
    },
    Ping {
        sequence: u64,
        event: ClientPingEvent,
    },
}

struct Progress {
    next: u64,
    completed: bool,
}
#[derive(uniffi::Object)]
pub struct ClientRequest {
    request: ipc::Request,
    progress: Mutex<Progress>,
}
#[uniffi::export(async_runtime = "tokio")]
impl ClientRequest {
    #[uniffi::constructor]
    pub fn new(request_id: String, command: ClientCommand) -> Result<Arc<Self>, ClientError> {
        let request_id: RequestId = request_id.parse().map_err(|_| ClientError::Input)?;
        Ok(Arc::new(Self {
            request: ipc::Request {
                version: ipc::VERSION,
                request_id,
                command: command.try_into()?,
            },
            progress: Mutex::new(Progress {
                next: 0,
                completed: false,
            }),
        }))
    }
    pub async fn frame(&self) -> Result<Vec<u8>, ClientError> {
        let mut frame = Vec::new();
        ipc::write(&mut frame, &self.request)
            .await
            .map_err(|_| ClientError::Frame)?;
        Ok(frame)
    }
    /// One complete XPC frame at a time. The caller serializes consumption and
    /// ACKs after it is decoded. Errors invalidate this request's stream.
    pub async fn decode(&self, frame: Vec<u8>) -> Result<ClientResult, ClientError> {
        let response = decode(&frame).await?;
        let mut progress = self.progress.lock().map_err(|_| ClientError::Completed)?;
        if progress.completed {
            return Err(ClientError::Completed);
        }
        progress.completed = true;
        if response.version != ipc::VERSION || response.request_id != self.request.request_id {
            return Err(ClientError::Contract);
        }
        match (&self.request.command, &response.result) {
            (_, ipc::ResultBody::Error { .. }) if progress.next == 0 => {}
            (ipc::Command::Ping { .. }, ipc::ResultBody::Ping { sequence, event })
                if sequence.get() == progress.next =>
            {
                progress.next = progress.next.checked_add(1).ok_or(ClientError::Contract)?;
                progress.completed = matches!(
                    event,
                    ipc::PingEvent::Completed { .. } | ipc::PingEvent::Failed { .. }
                );
            }
            (ipc::Command::Netcheck, ipc::ResultBody::Netcheck { .. }) => {}
            (
                ipc::Command::SelectProfile { .. } | ipc::Command::ClearProfile,
                ipc::ResultBody::Selection { .. },
            ) => {}
            (
                ipc::Command::Status
                | ipc::Command::Up { .. }
                | ipc::Command::Down { .. }
                | ipc::Command::RotateKey { .. }
                | ipc::Command::Logout { .. },
                ipc::ResultBody::Status { .. },
            ) => {}
            _ => return Err(ClientError::Contract),
        }
        Ok(match response.result {
            ipc::ResultBody::Status { status } => ClientResult::Status {
                status: (*status).into(),
            },
            ipc::ResultBody::Error { code } => ClientResult::Error {
                message: code.to_string(),
                code: code.into(),
            },
            ipc::ResultBody::Selection { selection } => ClientResult::Selection {
                selection: (*selection).into(),
            },
            ipc::ResultBody::Netcheck { report } => ClientResult::Netcheck {
                report: (*report).into(),
            },
            ipc::ResultBody::Ping { sequence, event } => ClientResult::Ping {
                sequence: sequence.get(),
                event: event.into(),
            },
        })
    }
}
async fn decode(frame: &[u8]) -> Result<ipc::Response, ClientError> {
    if frame.len() > ipc::MAX_MESSAGE_BYTES + 4 {
        return Err(ClientError::Frame);
    }
    let mut bytes = frame;
    let response = ipc::read(&mut bytes)
        .await
        .map_err(|_| ClientError::Frame)?;
    if !bytes.is_empty() {
        return Err(ClientError::Frame);
    }
    Ok(response)
}

/// Lab stdout only: validated complete protocol data, never parsed by Swift.
#[uniffi::export(async_runtime = "tokio")]
pub async fn client_response_text(frame: Vec<u8>) -> Result<String, ClientError> {
    serde_json::to_string(&decode(&frame).await?).map_err(|_| ClientError::Frame)
}
