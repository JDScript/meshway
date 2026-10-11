use crate::configuration::ConfigurationBindings;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_configuration::ConfigurationDocument;
use meshway_model::{NetworkId, Revision};
use rusqlite::Transaction;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub(crate) struct Snapshot {
    pub(crate) network_id: NetworkId,
    pub(crate) revision: Revision,
    pub(crate) document: ConfigurationDocument,
    pub(crate) bindings: ConfigurationBindings,
}

/// Immutable complete versions only. The cache never answers authorization or
/// currentRevision. A hit also requires the exact SQL ciphertext fingerprint:
/// a transaction rollback followed by revision reuse cannot poison this cache.
#[derive(Default)]
pub(crate) struct SnapshotCache {
    entries: Mutex<BTreeMap<(NetworkId, Revision), CacheEntry>>,
}
struct CacheEntry {
    snapshot: Arc<Snapshot>,
    encoded_size: usize,
    ciphertext_digest: [u8; 32],
}
impl SnapshotCache {
    /// The process's one cache: every reader of immutable versions shares it.
    pub(crate) fn shared() -> Arc<Self> {
        static SHARED: std::sync::LazyLock<Arc<SnapshotCache>> =
            std::sync::LazyLock::new(Arc::default);
        Arc::clone(&SHARED)
    }

    pub(crate) fn load(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        sealer: &Sealer,
    ) -> Result<Arc<Snapshot>, Error> {
        let sealed = crate::store::configuration::versions::load(tx, network, revision)?;
        let fingerprint = sealed.fingerprint();
        if let Some(entry) = self
            .entries
            .lock()
            .map_err(|_| Error::RUNTIME)?
            .get(&(network, revision))
            && entry.ciphertext_digest == fingerprint
        {
            return Ok(Arc::clone(&entry.snapshot));
        }
        let size = sealed.encoded_size();
        let snapshot = Arc::new(sealed.open(sealer)?);
        let mut entries = self.entries.lock().map_err(|_| Error::RUNTIME)?;
        const MAX_BYTES: usize = 64 * 1024 * 1024;
        if size <= MAX_BYTES {
            while entries.len() >= 32
                || entries
                    .values()
                    .map(|entry| entry.encoded_size)
                    .sum::<usize>()
                    + size
                    > MAX_BYTES
            {
                entries.pop_first();
            }
            entries.insert(
                (network, revision),
                CacheEntry {
                    snapshot: Arc::clone(&snapshot),
                    encoded_size: size,
                    ciphertext_digest: fingerprint,
                },
            );
        }
        Ok(snapshot)
    }
}
