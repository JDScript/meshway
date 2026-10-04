//! Persist operator intent and idempotency receipts before publishing a new plan.
use crate::ipc::SocketIdentity;
use meshway_ipc::ErrorCode;
use meshway_model::{RequestId, Version};
use meshway_private_storage::Directory;
use serde::{Deserialize, Serialize};
use std::{io::Read, sync::Arc};
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct Desired {
    pub revision: Version,
    pub up: bool,
    pub shutdown: bool,
    pub retry_enrollment: bool,
}

pub(crate) struct Mutation {
    pub action: Action,
    pub id: RequestId,
    pub expected: Version,
    pub reply: oneshot::Sender<Result<(), ErrorCode>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Action {
    Up,
    Down,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    id: RequestId,
    action: Action,
    expected_revision: Version,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    version: u8,
    pub revision: Version,
    pub desired_up: bool,
    #[serde(deserialize_with = "Option::deserialize")]
    last_request: Option<Receipt>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub socket: Option<SocketIdentity>,
}

pub(super) struct Store {
    pub directory: Arc<Directory>,
    pub record: Record,
}
impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, &'static str> {
        let directory = Arc::new(
            Directory::open(path, "daemon.lock")
                .map_err(|_| "Node directory ownership or lock failed.")?,
        );
        let record = if let Some(file) = directory
            .file("daemon.json", false)
            .map_err(|_| "Invalid Node state file.")?
        {
            let mut bytes = Vec::new();
            file.take(16_385)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read Node state.")?;
            if bytes.len() > 16_384 {
                return Err("Node state exceeds its limit.");
            }
            let mut decoder = serde_json::Deserializer::from_slice(&bytes);
            let record = Record::deserialize(meshway_serde::MapOnly(&mut decoder))
                .map_err(|_| "Invalid Node state; file preserved.")?;
            decoder
                .end()
                .map_err(|_| "Invalid Node state; file preserved.")?;
            if record.version != 1 {
                return Err("Unsupported Node state version.");
            }
            record
        } else {
            for entry in
                std::fs::read_dir(directory.path()).map_err(|_| "Cannot inspect Node directory.")?
            {
                if entry
                    .map_err(|_| "Cannot inspect Node directory.")?
                    .file_name()
                    != "daemon.lock"
                {
                    return Err("Node state is missing from a nonempty directory.");
                }
            }
            let record = Record {
                version: 1,
                revision: Version::MIN,
                desired_up: false,
                last_request: None,
                socket: None,
            };
            save(&directory, &record)?;
            record
        };
        Ok(Self { directory, record })
    }

    pub fn mutation(
        &self,
        action: Action,
        id: RequestId,
        expected: Version,
        blocked: bool,
    ) -> Result<Option<Record>, ErrorCode> {
        if let Some(last) = &self.record.last_request
            && last.id == id
        {
            return if last.action == action && last.expected_revision == expected {
                Ok(None)
            } else {
                Err(ErrorCode::RequestConflict)
            };
        }
        if expected != self.record.revision {
            return Err(ErrorCode::Conflict);
        }
        let mut record = self.record.clone();
        let desired = action == Action::Up;
        if desired != record.desired_up || desired && blocked {
            record.revision = Version::new(
                record
                    .revision
                    .get()
                    .checked_add(1)
                    .ok_or(ErrorCode::Storage)?,
            )
            .map_err(|_| ErrorCode::Storage)?;
        }
        record.desired_up = desired;
        record.last_request = Some(Receipt {
            id,
            action,
            expected_revision: expected,
        });
        Ok(Some(record))
    }

    /// The coordinator awaits this accepted commit, independent of the IPC waiter.
    pub async fn commit(&mut self, record: Record) -> Result<(), &'static str> {
        let directory = Arc::clone(&self.directory);
        let saved = record.clone();
        tokio::task::spawn_blocking(move || save(&directory, &saved))
            .await
            .map_err(|_| "Node state worker failed.")??;
        self.record = record;
        Ok(())
    }
    pub fn socket(&mut self, socket: SocketIdentity) -> Result<(), &'static str> {
        self.record.socket = Some(socket);
        save(&self.directory, &self.record)
    }
}
fn save(directory: &Directory, record: &Record) -> Result<(), &'static str> {
    let bytes = serde_json::to_vec(record).map_err(|_| "Cannot encode Node state.")?;
    directory
        .replace("daemon.json", &bytes)
        .map_err(|_| "Cannot commit Node state; reopen before retrying.")
}
