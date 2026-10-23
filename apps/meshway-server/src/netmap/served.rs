//! The rollback boundary of an upgrade, persisted: before this process sends
//! its first sync answer (from which a Node may hold a lease it signed), it
//! durably replaces `served` in the data directory with a record naming this
//! process (`boot`, a new identifier per process) and its build. An answer is
//! never sent without the record; a record whose write outcome is unknown
//! fails the answer, and the next sync writes it again.
//!
//! `install.sh` reads the file before it starts a new Server and again after
//! a failed start: any difference (or an unreadable file) means the new
//! Server may have issued a lease, and the upgrade is not rolled back. The
//! log line `First Node sync answered by this Server process` is diagnostics
//! only.

use crate::{directory::Directory, error::Error};
use meshway_model::InstanceId;

pub(crate) struct Served {
    /// The SQLite worker owns the directory and its lock; a record is written
    /// only while it does, so handles outliving the App keep no lock.
    directory: std::sync::Weak<Directory>,
    boot: InstanceId,
    recorded: tokio::sync::OnceCell<()>,
}

impl Served {
    pub(crate) fn new(directory: std::sync::Weak<Directory>, boot: InstanceId) -> Self {
        Self {
            directory,
            boot,
            recorded: tokio::sync::OnceCell::new(),
        }
    }

    /// Returns once the record of this process is durable; before the first
    /// success every caller waits for (or repeats) the one write.
    pub(crate) async fn ensure(&self) -> Result<(), Error> {
        self.recorded
            .get_or_try_init(|| async {
                let directory = self.directory.upgrade().ok_or(Error::CLOSED)?;
                let record = record(self.boot);
                tokio::task::spawn_blocking(move || directory.record_served(record.as_bytes()))
                    .await
                    .map_err(|_| Error::RUNTIME)?
                    .map_err(|error| {
                        tracing::error!(%error, "Served record not written; the sync is not answered");
                        Error::Unavailable(crate::error::Unavailability::ServedRecord)
                    })
            })
            .await
            .map(|_| ())
    }
}

fn record(boot: InstanceId) -> String {
    format!(
        "meshway-server served\nboot={}\nbuild={}\nat_ms={}\n",
        boot.as_uuid(),
        meshway_build::identity(),
        chrono::Utc::now().timestamp_millis(),
    )
}
