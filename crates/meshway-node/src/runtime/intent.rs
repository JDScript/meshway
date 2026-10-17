//! Persist operator intent and idempotency receipts before publishing a new plan.
use crate::ipc::SocketIdentity;
use meshway_model::{RequestId, Version};
use meshway_private_storage::Directory;
use meshway_protocol::ipc::{EnrollmentKey, ErrorCode};
use serde::{Deserialize, Serialize};
use std::{io::Read, os::unix::fs::FileTypeExt, sync::Arc};
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct Desired {
    pub revision: Version,
    pub up: bool,
    pub shutdown: bool,
    pub retry_enrollment: bool,
    pub enrollment_key: Option<EnrollmentKey>,
    pub rotation: Option<RequestId>,
    /// The operator asked to enroll the held identity again; transient,
    /// Control holds the started request durably.
    pub reenroll: bool,
    /// A requested self-deletion awaiting its outcome.
    pub logout: Option<RequestId>,
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
    Reenroll,
    Logout,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logout: Option<Rotation>,
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
            reenroll: false,
            logout: self
                .logout
                .as_ref()
                .filter(|logout| logout.result.is_none())
                .map(|logout| logout.id),
        }
    }
}

const LOCK: &str = "daemon.lock";
const RECORD: &str = "daemon.json";

/// Whether `name` in the state directory belongs to this daemon or to a
/// component it assembles there. Each component's leftovers (a stale IPC or
/// bootstrap socket, a resource journal of a previous instance and its
/// archives, the Control identity directory) are recovered by that component
/// under its own rules once the daemon runs; a missing intent record never
/// turns them into a reason to refuse starting. Anything else is storage of
/// unknown ownership and keeps the directory closed.
fn owned(name: &str) -> bool {
    name == LOCK
        || name == RECORD
        || name == crate::ipc::SOCKET_FILE
        || name == super::control::IDENTITY_DIRECTORY
        || meshway_underlay::owns_file(name)
        || meshway_private_storage::is_staging(name)
}

pub(super) struct Store {
    pub directory: Arc<Directory>,
    pub record: Record,
}
impl Store {
    pub fn open(path: &std::path::Path) -> Result<Self, &'static str> {
        let directory = Arc::new(
            Directory::open(path, LOCK).map_err(|_| "Node directory ownership or lock failed.")?,
        );
        let record = if let Some(file) = directory
            .file(RECORD, false)
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
            // A new or emptied installation: the intent record is created only
            // among this daemon's own files, each of its own kind.
            let mut leftovers = Vec::new();
            for entry in
                std::fs::read_dir(directory.path()).map_err(|_| "Cannot inspect Node directory.")?
            {
                let entry = entry.map_err(|_| "Cannot inspect Node directory.")?;
                let kind = entry
                    .file_type()
                    .map_err(|_| "Cannot inspect Node directory.")?;
                let name = entry.file_name();
                let own = name.to_str().is_some_and(|name| {
                    owned(name)
                        && if name == super::control::IDENTITY_DIRECTORY {
                            kind.is_dir()
                        } else {
                            kind.is_file() || kind.is_socket()
                        }
                });
                if !own {
                    return Err("Node state is missing from a directory with unowned files.");
                }
                if name != LOCK {
                    leftovers.push(name.to_string_lossy().into_owned());
                }
            }
            if !leftovers.is_empty() {
                leftovers.sort();
                tracing::warn!(
                    files = ?leftovers,
                    "Node state is missing; starting unenrolled among this daemon's own files"
                );
            }
            let record = Record {
                version: 1,
                revision: Version::MIN,
                desired_up: false,
                last_request: None,
                socket: None,
                enrollment_key: None,
                rotation: None,
                logout: None,
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
        let rotating = self
            .record
            .rotation
            .as_ref()
            .is_some_and(|rotation| rotation.result.is_none());
        let leaving = self
            .record
            .logout
            .as_ref()
            .is_some_and(|logout| logout.result.is_none());
        if (rotating && action != Action::Down) || leaving {
            return Err(ErrorCode::Busy);
        }
        if matches!(
            action,
            Action::RotateKey | Action::Reenroll | Action::Logout
        ) && self.record.enrollment_key.is_some()
        {
            return Err(ErrorCode::NotReady);
        }
        let mut record = self.record.clone();
        let desired = match action {
            Action::Up | Action::Reenroll => true,
            Action::Down => false,
            // Down only once the Server confirmed the deletion.
            Action::RotateKey | Action::Logout => record.desired_up,
        };
        // A new operator request retries failed convergence in either direction.
        // The receipt check above still makes an uncertain request replay inert.
        if desired != record.desired_up
            || blocked
            || enrollment_key.is_some()
            || matches!(
                action,
                Action::RotateKey | Action::Reenroll | Action::Logout
            )
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
            Action::Logout => record.logout = Some(Rotation { id, result: None }),
            Action::Reenroll => {}
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
        .replace(RECORD, &bytes)
        .map_err(|_| "Cannot commit Node state; reopen before retrying.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The journal `meshway-vm` was left with after its identity files were
    /// removed by hand (F3 acceptance leftover): same boot and namespace, no
    /// recorded resources.
    const EMPTY_JOURNAL: &[u8] = br#"{"version":1,"boot":"f8871843-30b4-4a12-ae10-15123abcf79b\n","namespace_device":5,"namespace_inode":4026532281,"installation":"3f78fbb7c9765599939ea6ce3c4e5b2e","binding":null,"mark":{"value":2147483648,"mask":2147483648},"table":3086720847,"start":6524,"pool_limit":64,"pools":[],"rules":[],"routes":[],"tun":null,"anchor":null,"pending":null}"#;
    /// A journal recording a negative capture boundary of a previous instance.
    /// Whether those rules still exist is the Host's question when the daemon
    /// opens it (`meshway_underlay` recovery); it is not a reason to refuse start.
    const RESOURCE_JOURNAL: &[u8] = br#"{"version":1,"boot":"f8871843-30b4-4a12-ae10-15123abcf79b\n","namespace_device":5,"namespace_inode":4026532281,"installation":"3f78fbb7c9765599939ea6ce3c4e5b2e","binding":null,"mark":{"value":2147483648,"mask":2147483648},"table":3086720847,"start":6524,"pool_limit":64,"pools":["100.64.0.0/10"],"rules":[{"spec":{"kind":"End","priority":6653,"table":3086720847,"ipv6":false},"observed":[1,2,3]}],"routes":[],"tun":null,"anchor":null,"pending":null}"#;

    struct Shape {
        files: Vec<(&'static str, &'static [u8])>,
        sockets: Vec<&'static str>,
        identity: bool,
    }

    fn shape() -> Shape {
        Shape {
            files: Vec::new(),
            sockets: Vec::new(),
            identity: false,
        }
    }

    fn lay_out(root: &std::path::Path, shape: &Shape) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("node");
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        for (name, bytes) in &shape.files {
            std::fs::write(path.join(name), bytes).unwrap();
            std::fs::set_permissions(path.join(name), std::fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        for name in &shape.sockets {
            drop(std::os::unix::net::UnixListener::bind(path.join(name)).unwrap());
        }
        if shape.identity {
            // The Control directory of a Node that left (`meshway logout`):
            // its lock marker, the unenrolled identity record and the clock floor.
            let identity = path.join(super::super::control::IDENTITY_DIRECTORY);
            std::fs::create_dir(&identity).unwrap();
            std::fs::set_permissions(&identity, std::fs::Permissions::from_mode(0o700)).unwrap();
            for (name, bytes) in [
                ("node-control.lock", &b"meshway-control/1\n"[..]),
                (
                    "control.json",
                    br#"{"version":5,"origin":"https://server.example","keys":null,"enrollment":null,"binding":null}"#,
                ),
                (
                    "clock.json",
                    br#"{"format":"meshway-clock-floor/1","highWaterMs":1760200000000}"#,
                ),
            ] {
                std::fs::write(identity.join(name), bytes).unwrap();
                std::fs::set_permissions(
                    identity.join(name),
                    std::fs::Permissions::from_mode(0o600),
                )
                .unwrap();
            }
        }
        path
    }

    fn opens_unenrolled(shape: &Shape) -> Result<Vec<String>, &'static str> {
        let temp = tempfile::tempdir().unwrap();
        let path = lay_out(&temp.path().canonicalize().unwrap(), shape);
        let before: std::collections::BTreeSet<_> = std::fs::read_dir(&path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let store = Store::open(&path)?;
        assert!(!store.record.desired_up);
        assert_eq!(store.record.revision, Version::MIN);
        assert!(store.record.enrollment_key.is_none());
        assert!(store.record.rotation.is_none() && store.record.logout.is_none());
        // Opening writes the record and touches nothing else.
        let after: std::collections::BTreeSet<_> = std::fs::read_dir(&path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let mut expected = before;
        expected.insert(LOCK.into());
        expected.insert(RECORD.into());
        assert_eq!(after, expected);
        Ok(after.into_iter().collect())
    }

    #[test]
    fn a_missing_record_starts_unenrolled_among_the_daemons_own_files() {
        // Empty directory and lock only: a new installation (also the shape
        // after `install.sh --uninstall --purge` and a reinstall).
        opens_unenrolled(&shape()).unwrap();
        let mut only_lock = shape();
        only_lock.files.push((LOCK, b""));
        opens_unenrolled(&only_lock).unwrap();

        // Lock and the empty journal of the previous instance (the F3 leftover).
        let mut empty_journal = shape();
        empty_journal.files.push((LOCK, b""));
        empty_journal.files.push(("underlay.json", EMPTY_JOURNAL));
        opens_unenrolled(&empty_journal).unwrap();

        // Lock and a journal recording resources, with the bounded stale
        // archives and the other Underlay records: the Host reclaims them.
        let mut resources = shape();
        resources.files.extend([
            (LOCK, &b""[..]),
            ("underlay.json", RESOURCE_JOURNAL),
            ("underlay.lock", b""),
            ("underlay.json.stale-1760100000000000000", EMPTY_JOURNAL),
            ("underlay.json.stale-1760100000000000001", EMPTY_JOURNAL),
            ("underlay.json.stale-1760100000000000002", EMPTY_JOURNAL),
            ("underlay-egress.json", b"{}"),
            ("underlay-exit.json", b"{}"),
            ("exit-child.json", b"{}"),
            ("underlay-bootstrap.json", br#"{"device":1,"inode":2}"#),
        ]);
        resources.sockets.push("underlay-bootstrap.sock");
        resources.sockets.push(crate::ipc::SOCKET_FILE);
        opens_unenrolled(&resources).unwrap();

        // The Control directory of a Node that left, next to the journal.
        let mut left = shape();
        left.files.push((LOCK, b""));
        left.files.push(("underlay.json", EMPTY_JOURNAL));
        left.identity = true;
        opens_unenrolled(&left).unwrap();

        // An interrupted private write's staging file is this daemon's own.
        let mut staging = shape();
        staging
            .files
            .push((".write-0123456789abcdef0123456789abcdef", b"{"));
        opens_unenrolled(&staging).unwrap();
    }

    #[test]
    fn a_missing_record_keeps_a_directory_with_unowned_files_closed() {
        let rejected = "Node state is missing from a directory with unowned files.";
        let mut unknown = shape();
        unknown.files.push((LOCK, b""));
        unknown.files.push(("underlay.json", EMPTY_JOURNAL));
        unknown.files.push(("notes.txt", b"operator"));
        assert_eq!(opens_unenrolled(&unknown), Err(rejected));

        let mut near_miss = shape();
        near_miss.files.push(("underlay.json.bak", EMPTY_JOURNAL));
        assert_eq!(opens_unenrolled(&near_miss), Err(rejected));

        let mut stale_dir = shape();
        stale_dir
            .files
            .push(("underlay.json.stale-", EMPTY_JOURNAL));
        assert_eq!(opens_unenrolled(&stale_dir), Err(rejected));

        // An owned name of the wrong kind is not the daemon's file.
        let temp = tempfile::tempdir().unwrap();
        let path = lay_out(&temp.path().canonicalize().unwrap(), &shape());
        std::os::unix::fs::symlink(path.join("elsewhere"), path.join("underlay.json")).unwrap();
        assert_eq!(Store::open(&path).err(), Some(rejected));
        std::fs::remove_file(path.join("underlay.json")).unwrap();
        std::fs::create_dir(path.join("underlay.json")).unwrap();
        assert_eq!(Store::open(&path).err(), Some(rejected));
        std::fs::remove_dir(path.join("underlay.json")).unwrap();
        std::fs::write(path.join(super::super::control::IDENTITY_DIRECTORY), b"").unwrap();
        assert_eq!(Store::open(&path).err(), Some(rejected));
        assert!(!path.join(RECORD).exists(), "no record was written");
    }

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
    async fn reenroll_is_an_up_with_its_own_revision_and_replay_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().canonicalize().unwrap().join("node")).unwrap();
        let up = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let record = store
            .mutation(Action::Up, up, store.record.revision, false, None)
            .unwrap()
            .unwrap();
        store.commit(record).await.unwrap();
        let revision = store.record.revision;
        let again = "00000000-0000-4000-8000-000000000002".parse().unwrap();
        let record = store
            .mutation(Action::Reenroll, again, revision, false, None)
            .unwrap()
            .unwrap();
        assert!(record.desired_up);
        assert_eq!(record.revision.get(), revision.get() + 1, "a new session");
        store.commit(record).await.unwrap();
        assert!(
            store
                .mutation(Action::Reenroll, again, revision, false, None)
                .unwrap()
                .is_none(),
            "an identical replay is inert"
        );
        assert!(matches!(
            store.mutation(Action::Up, again, revision, false, None),
            Err(ErrorCode::RequestConflict)
        ));
        assert!(!store.record.desired(true).reenroll, "never durable here");
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
