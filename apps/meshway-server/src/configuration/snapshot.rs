use super::*;
use sha2::{Digest as _, Sha256};
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
    pub(crate) fn load(
        &self,
        tx: &Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        sealer: &Sealer,
    ) -> Result<Arc<Snapshot>, Error> {
        let (source, configuration, bindings) = tx.query_row(
            "SELECT source_jsonc,configuration,bindings FROM configuration_versions WHERE network_id=?1 AND revision=?2",
            params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
        ).optional()?.ok_or(Error::NotFound)?;
        let mut hash = Sha256::new();
        for bytes in [&source, &configuration, &bindings] {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        let fingerprint: [u8; 32] = hash.finalize().into();
        if let Some(entry) = self
            .entries
            .lock()
            .map_err(|_| Error::RUNTIME)?
            .get(&(network, revision))
            && entry.ciphertext_digest == fingerprint
        {
            return Ok(Arc::clone(&entry.snapshot));
        }
        let size = source.len() + configuration.len() + bindings.len();
        let source = sealer.open(network, revision, Purpose::Source, &source)?;
        let configuration =
            sealer.open(network, revision, Purpose::Configuration, &configuration)?;
        let bindings = sealer.open(network, revision, Purpose::Bindings, &bindings)?;
        let source = std::str::from_utf8(source.as_bytes())
            .map_err(|_| Error::decode("configuration_versions.source_jsonc"))?;
        meshway_configuration::parse_jsonc(source)
            .map_err(|_| Error::decode("configuration_versions.source_jsonc"))?;
        let configuration = meshway_configuration::restore_canonical_json(
            std::str::from_utf8(configuration.as_bytes())
                .map_err(|_| Error::decode("configuration_versions.configuration"))?,
        )
        .map_err(|_| Error::decode("configuration_versions.configuration"))?;
        let bindings: ConfigurationBindings = serde_json::from_slice(bindings.as_bytes())
            .map_err(|_| Error::decode("configuration_versions.bindings"))?;
        let snapshot = Arc::new(Snapshot {
            network_id: network,
            revision,
            document: ConfigurationDocument::new(source.to_owned(), configuration),
            bindings,
        });
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
