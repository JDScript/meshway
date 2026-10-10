//! Persist operator intent and idempotency receipts before publishing a new plan.
use crate::ipc::SocketIdentity;
use meshway_ipc::{EnrollmentKey, ErrorCode};
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
    pub enrollment_key: Option<EnrollmentKey>,
    pub rotation: Option<RequestId>,
}

pub(crate) struct Mutation {
    pub action: Action,
    pub id: RequestId,
    pub expected: Version,
    pub reply: oneshot::Sender<Result<(), ErrorCode>>,
    pub enrollment_key: Option<EnrollmentKey>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Action {
    Up,
    Down,
    RotateKey,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    id: RequestId,
    action: Action,
    expected_revision: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_digest: Option<meshway_model::Digest>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Rotation {
    pub id: RequestId,
    #[serde(deserialize_with = "Option::deserialize")]
    pub result: Option<Result<(), ErrorCode>>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment_key: Option<EnrollmentKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
}
impl Record {
    pub fn desired(&self, retry_enrollment: bool) -> Desired {
        Desired {
            revision: self.revision,
            up: self.desired_up,
            shutdown: false,
            retry_enrollment,
            enrollment_key: self.enrollment_key.clone(),
            rotation: self
                .rotation
                .as_ref()
                .filter(|rotation| rotation.result.is_none())
                .map(|rotation| rotation.id),
        }
    }
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
            let record = Record::deserialize(meshway_model::wire::MapOnly(&mut decoder))
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
                enrollment_key: None,
                rotation: None,
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
        enrollment_key: Option<EnrollmentKey>,
    ) -> Result<Option<Record>, ErrorCode> {
        let key_digest = enrollment_key
            .as_ref()
            .map(meshway_protocol::node::request_digest)
            .transpose()
            .map_err(|_| ErrorCode::Storage)?;
        if enrollment_key.is_some() && action != Action::Up {
            return Err(ErrorCode::Contract);
        }
        if let Some(last) = &self.record.last_request
            && last.id == id
        {
            return if last.action == action
                && last.expected_revision == expected
                && last.key_digest == key_digest
            {
                Ok(None)
            } else {
                Err(ErrorCode::RequestConflict)
            };
        }
        if expected != self.record.revision {
            return Err(ErrorCode::Conflict);
        }
        if self
            .record
            .rotation
            .as_ref()
            .is_some_and(|rotation| rotation.result.is_none())
            && action != Action::Down
        {
            return Err(ErrorCode::Busy);
        }
        if action == Action::RotateKey && self.record.enrollment_key.is_some() {
            return Err(ErrorCode::NotReady);
        }
        let mut record = self.record.clone();
        let desired = match action {
            Action::Up => true,
            Action::Down => false,
            Action::RotateKey => record.desired_up,
        };
        // A new operator request retries failed convergence in either direction.
        // The receipt check above still makes an uncertain request replay inert.
        if desired != record.desired_up
            || blocked
            || enrollment_key.is_some()
            || action == Action::RotateKey
        {
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
        match action {
            Action::Up => {
                if enrollment_key.is_some() {
                    record.enrollment_key = enrollment_key;
                }
            }
            Action::Down => record.enrollment_key = None,
            Action::RotateKey => record.rotation = Some(Rotation { id, result: None }),
        }
        record.last_request = Some(Receipt {
            id,
            action,
            expected_revision: expected,
            key_digest,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn key_intent_replay_uses_digest_after_secret_retirement_and_rotation_preserves_down() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().canonicalize().unwrap().join("node")).unwrap();
        let initial = store.record.revision;
        let id = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let key = EnrollmentKey::new("private-fixture-key".into()).unwrap();
        let record = store
            .mutation(Action::Up, id, initial, false, Some(key.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(
            record.enrollment_key.as_ref().unwrap().expose(),
            key.expose()
        );
        store.commit(record).await.unwrap();
        let mut record = store.record.clone();
        record.enrollment_key = None;
        store.commit(record).await.unwrap();
        assert!(
            store
                .mutation(Action::Up, id, initial, false, Some(key))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            store.mutation(Action::Up, id, initial, false, None),
            Err(ErrorCode::RequestConflict)
        ));
        let down = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let record = store
            .mutation(Action::Down, down, store.record.revision, false, None)
            .unwrap()
            .unwrap();
        store.commit(record).await.unwrap();
        let rotate = "00000000-0000-4000-8000-000000000003".parse().unwrap();
        let revision = store.record.revision;
        let record = store
            .mutation(Action::RotateKey, rotate, revision, false, None)
            .unwrap()
            .unwrap();
        assert!(!record.desired_up);
        store.commit(record).await.unwrap();
        assert!(
            store
                .mutation(Action::RotateKey, rotate, revision, false, None)
                .unwrap()
                .is_none()
        );
        assert_eq!(store.record.desired(false).rotation, Some(rotate));
        let mut record = store.record.clone();
        record.rotation.as_mut().unwrap().result = Some(Ok(()));
        store.commit(record).await.unwrap();
        assert_eq!(store.record.desired(false).rotation, None);
    }

    #[tokio::test]
    async fn blocked_down_advances_once_and_preserves_cas_and_durable_replay() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("node");
        let mut store = Store::open(&path).unwrap();
        let before = store.record.revision;
        let id = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let record = store
            .mutation(Action::Down, id, before, true, None)
            .unwrap()
            .unwrap();
        assert!(!record.desired_up);
        assert_eq!(record.revision.get(), before.get() + 1);
        store.commit(record).await.unwrap();
        drop(store);

        let store = Store::open(&path).unwrap();
        assert!(
            store
                .mutation(Action::Down, id, before, true, None)
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            store.mutation(Action::Up, id, before, true, None),
            Err(ErrorCode::RequestConflict)
        ));
        let next = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        assert!(matches!(
            store.mutation(Action::Down, next, before, true, None),
            Err(ErrorCode::Conflict)
        ));
        let healthy = store
            .mutation(Action::Down, next, store.record.revision, false, None)
            .unwrap()
            .unwrap();
        assert_eq!(healthy.revision, store.record.revision);
        assert!(!healthy.desired_up);
    }
}
