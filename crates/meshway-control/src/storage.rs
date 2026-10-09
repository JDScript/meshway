//! Private Node control storage.
//!
//! `control.json` (record v5) holds only the identity envelope: keys, pending
//! enrollment, binding and rotation. Derived caches live in their own files
//! with their own format identifiers, so compiler changes never touch the
//! identity version: `netmap.json` (see `netmap::store`) and the legacy
//! publication target `execution.json` (removed with the legacy chain).
use crate::{
    Control, Error,
    identity::{self, Keys},
};
use meshway_configuration::execution::{CompiledNode, validate_obsolete_slice};
use meshway_protocol::{EnrollmentIntent, EnrollmentResponse, WireSecret};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use std::{
    io::{Read, Write},
    sync::Arc,
};
use zeroize::Zeroizing;

pub(super) const LIMIT: usize = 16 * 1024 * 1024;
const RECORD: &str = "control.json";
const LEGACY_CACHE: &str = "execution.json";
const LEGACY_FORMAT: &str = "meshway-execution-cache/1";
const MARKER: &[u8] = b"meshway-control/1\n";
const VERSION: u8 = 5;

fn marker(directory: &meshway_private_storage::Directory) -> Result<bool, Error> {
    let file = directory
        .file("node-control.lock", false)?
        .ok_or(Error::Record)?;
    let mut bytes = Vec::new();
    file.take((MARKER.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| meshway_private_storage::Error::Io)?;
    if bytes.is_empty() {
        Ok(false)
    } else if bytes == MARKER {
        Ok(true)
    } else {
        Err(Error::Record)
    }
}
pub(super) fn ensure_marker(directory: &meshway_private_storage::Directory) -> Result<(), Error> {
    let mut file = directory
        .file("node-control.lock", false)?
        .ok_or(Error::Record)?;
    if !marker(directory)? {
        file.write_all(MARKER)
            .map_err(|_| meshway_private_storage::Error::Io)?;
    }
    file.sync_all()
        .map_err(|_| meshway_private_storage::Error::Io)?;
    directory.sync()?;
    Ok(())
}

/// Identity envelope v5. Derived caches are never part of it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    version: u8,
    origin: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub keys: Option<Keys>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub enrollment: Option<Enrollment>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub binding: Option<EnrollmentResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<crate::rotation::Rotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_rotation: Option<meshway_model::RequestId>,
}

/// Records v1–v4 carried a derived target in `saved`. It is inspected only for
/// integrity and binding, archived with the original bytes, and dropped.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Previous<'a> {
    version: u8,
    origin: String,
    #[serde(deserialize_with = "Option::deserialize")]
    keys: Option<Keys>,
    #[serde(deserialize_with = "Option::deserialize")]
    enrollment: Option<Enrollment>,
    #[serde(deserialize_with = "Option::deserialize")]
    binding: Option<EnrollmentResponse>,
    #[serde(borrow, deserialize_with = "Option::deserialize")]
    saved: Option<&'a RawValue>,
    #[serde(default)]
    rotation: Option<crate::rotation::Rotation>,
    #[serde(default)]
    completed_rotation: Option<meshway_model::RequestId>,
}

#[derive(Deserialize)]
struct Version {
    version: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Enrollment {
    pub intent: EnrollmentIntent,
    pub verifier: WireSecret,
    #[serde(deserialize_with = "Option::deserialize")]
    pub code: Option<WireSecret>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub approved_network: Option<meshway_model::NetworkId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key: Option<WireSecret>,
}

/// Private bytes of one bounded file, or `None` when absent.
pub(super) fn read(
    directory: &meshway_private_storage::Directory,
    name: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    let Some(file) = directory.file(name, false)? else {
        return Ok(None);
    };
    let mut bytes = Zeroizing::new(Vec::with_capacity(LIMIT.min(1 << 20)));
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| meshway_private_storage::Error::Io)?;
    if bytes.len() > LIMIT {
        return Err(Error::Record);
    }
    Ok(Some(bytes))
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut buffer = Buffer(Zeroizing::new(Vec::new()));
    serde_json::to_writer(&mut buffer, value).map_err(|_| Error::Record)?;
    Ok(buffer.0)
}

impl Record {
    pub fn load(
        directory: &meshway_private_storage::Directory,
        origin: &str,
    ) -> Result<Self, Error> {
        let initialized = marker(directory)?;
        let Some(file) = directory.file(RECORD, false)? else {
            if initialized {
                return Err(Error::Record);
            }
            // An absent identity record is only a new installation in an empty
            // domain directory. Preserve unknown files and crash staging data.
            for entry in std::fs::read_dir(directory.path())
                .map_err(|_| meshway_private_storage::Error::Io)?
            {
                if entry
                    .map_err(|_| meshway_private_storage::Error::Io)?
                    .file_name()
                    != "node-control.lock"
                {
                    return Err(Error::Record);
                }
            }
            return Ok(Self {
                version: VERSION,
                origin: origin.into(),
                keys: None,
                enrollment: None,
                binding: None,
                rotation: None,
                completed_rotation: None,
            });
        };
        file.sync_all()
            .map_err(|_| meshway_private_storage::Error::Io)?;
        directory.sync()?;
        let bytes = read(directory, RECORD)?.ok_or(Error::Record)?;
        let Version { version } = serde_json::from_slice(&bytes).map_err(|_| Error::Record)?;
        let record = match version {
            VERSION => {
                let record: Self = decode(&bytes)?;
                if record.origin != origin {
                    return Err(Error::Record);
                }
                record.validate()?;
                record
            }
            1..=4 => {
                let previous: Previous<'_> = decode(&bytes)?;
                if previous.origin != origin || previous.version != version {
                    return Err(Error::Record);
                }
                if let Some(saved) = previous.saved {
                    validate_obsolete(
                        saved,
                        previous.binding.as_ref().ok_or(Error::Record)?,
                        version,
                    )?;
                }
                let record = Self {
                    version: VERSION,
                    origin: previous.origin,
                    keys: previous.keys,
                    enrollment: previous.enrollment,
                    binding: previous.binding,
                    rotation: previous.rotation,
                    completed_rotation: previous.completed_rotation,
                };
                record.validate()?;
                archive(directory, version, &bytes)?;
                // The lock remains held and both writes are synchronous and
                // durable. A failed archive never replaces the original record.
                directory.replace(RECORD, &encode(&record)?)?;
                record
            }
            _ => return Err(Error::Record),
        };
        ensure_marker(directory)?;
        Ok(record)
    }

    fn validate(&self) -> Result<(), Error> {
        let record = self;
        if record.completed_rotation.is_some() && record.binding.is_none() {
            return Err(Error::Record);
        }
        if let Some(rotation) = &record.rotation {
            rotation.validate(record.binding.as_ref().ok_or(Error::Record)?)?;
        }
        match (&record.keys, &record.enrollment, &record.binding) {
            (None, None, None) => {}
            (Some(keys), Some(pending), None) => {
                if pending.intent.existing.is_some()
                    || pending.intent.control_key != keys.control()?
                    || pending.intent.dataplane_key != keys.dataplane()?
                    || pending.intent.code_challenge != identity::challenge(&pending.verifier)
                    || pending.code.is_some() != pending.approved_network.is_some()
                    || (pending.access_key.is_some() && pending.code.is_some())
                {
                    return Err(Error::Record);
                }
            }
            (Some(keys), None, Some(binding)) if keys.matches(binding)? => {}
            _ => return Err(Error::Record),
        }
        Ok(())
    }
}

pub(super) fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(meshway_serde::MapOnly(&mut decoder)).map_err(|_| Error::Record)?;
    decoder.end().map_err(|_| Error::Record)?;
    Ok(value)
}

/// Derived bodies of retired record versions (v1: demand-4 without pools, v2:
/// demand-4, v3: demand-5, v4: demand-7). MapOnly recursively rejects
/// duplicate keys, including inside fields that are not execution inputs
/// anymore. Header values retain their original strong types.
fn validate_obsolete(
    raw: &RawValue,
    binding: &EnrollmentResponse,
    version: u8,
) -> Result<(), Error> {
    if version == 4 {
        let saved: CompiledNode = decode(raw.get().as_bytes())?;
        return crate::execution::validate_target(&saved, binding).map_err(|_| Error::Record);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Slice<'a> {
        digest: meshway_model::Digest,
        #[serde(borrow)]
        configuration: &'a RawValue,
    }
    let slice: Slice<'_> = decode(raw.get().as_bytes())?;
    let fields: std::collections::BTreeMap<String, &RawValue> =
        decode(slice.configuration.get().as_bytes())?;
    if fields.contains_key("meshAddressPools") != (version >= 2) {
        return Err(Error::Record);
    }
    let header =
        validate_obsolete_slice(slice.configuration, slice.digest).map_err(|_| Error::Record)?;
    if header.network_id != binding.network_id
        || header.node_id != binding.node_id
        || header.self_keys.control_key != binding.keys.control
        || header.self_keys.dataplane_key != binding.keys.dataplane
    {
        return Err(Error::Record);
    }
    Ok(())
}

fn archive(
    directory: &meshway_private_storage::Directory,
    version: u8,
    bytes: &[u8],
) -> Result<(), Error> {
    let digest = meshway_model::Digest::from_bytes(Sha256::digest(bytes).into());
    let name = format!("control-v{version}-{digest}.json");
    if let Some(existing) = read(directory, &name)? {
        if existing.as_slice() != bytes {
            return Err(Error::Record);
        }
    } else {
        directory.replace(&name, bytes)?;
    }
    directory.sync()?;
    Ok(())
}

struct Buffer(Zeroizing<Vec<u8>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > LIMIT - self.0.len() {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
struct LegacyCache<T> {
    format: String,
    #[serde(deserialize_with = "Option::deserialize")]
    saved: Option<T>,
}

/// Legacy publication target cache. A missing, malformed, foreign-format or
/// misbound file is not authority and not identity: it is ignored and later
/// replaced by the next synchronized target.
pub(super) fn load_legacy(
    directory: &meshway_private_storage::Directory,
    binding: Option<&EnrollmentResponse>,
) -> Result<Option<CompiledNode>, Error> {
    let (Some(bytes), Some(binding)) = (read(directory, LEGACY_CACHE)?, binding) else {
        return Ok(None);
    };
    let Ok(LegacyCache::<CompiledNode> { format, saved }) = decode(&bytes) else {
        return Ok(None);
    };
    Ok(saved.filter(|saved| {
        format == LEGACY_FORMAT && crate::execution::validate_target(saved, binding).is_ok()
    }))
}

impl Control {
    /// Identity envelope write. Stops current execution authority first.
    pub(super) async fn persist(&mut self) -> Result<(), Error> {
        self.ready()?;
        self.invalidate_status_context();
        self.invalidate_snapshots();
        self.revoke_netmap();
        let bytes = encode(&self.record)?;
        self.write_private(RECORD, bytes).await
    }

    /// Only the validated configuration-query transaction may preserve lineage.
    pub(super) async fn persist_target(&mut self) -> Result<(), Error> {
        self.ready()?;
        self.invalidate_guards();
        let bytes = encode(&LegacyCache {
            format: LEGACY_FORMAT.into(),
            saved: self.saved.as_ref(),
        })?;
        self.write_private(LEGACY_CACHE, bytes).await
    }

    /// One fenced durable write under the single driver. Until fsync is
    /// acknowledged, neither this driver nor a cancelled caller may use the new
    /// state; an unknown outcome requires reopening the storage.
    pub(super) async fn write_private(
        &mut self,
        name: &'static str,
        bytes: Zeroizing<Vec<u8>>,
    ) -> Result<(), Error> {
        self.ready()?;
        self.dirty = true;
        // A cancelled caller leaves the outcome unknown: the fuse then cancels
        // every netmap guard, as an error does below.
        let fuse = self.netmap.fuse();
        let directory = Arc::clone(&self.directory);
        self.write = Some(tokio::task::spawn_blocking(move || {
            ensure_marker(&directory).map_err(|_| meshway_private_storage::Error::UnsafeStorage)?;
            directory.replace(name, &bytes)
        }));
        let result = self
            .write
            .as_mut()
            .ok_or(Error::ReloadRequired)?
            .await
            .map_err(|_| Error::ReloadRequired)?;
        self.write.take();
        if let Err(error) = result {
            // Unknown outcome: no snapshot may outlive it.
            drop(fuse);
            self.revoke_netmap();
            return Err(error.into());
        }
        fuse.disarm();
        self.dirty = false;
        Ok(())
    }
}
