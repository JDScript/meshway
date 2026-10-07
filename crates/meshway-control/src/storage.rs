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
const MARKER: &[u8] = b"meshway-control/1\n";
const VERSION: u8 = 2;

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
fn ensure_marker(directory: &meshway_private_storage::Directory) -> Result<(), Error> {
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub(super) struct Record<T = CompiledNode> {
    version: u8,
    origin: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub keys: Option<Keys>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub enrollment: Option<Enrollment>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub binding: Option<EnrollmentResponse>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub saved: Option<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<crate::rotation::Rotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_rotation: Option<meshway_model::RequestId>,
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
                saved: None,
                rotation: None,
                completed_rotation: None,
            });
        };
        file.sync_all()
            .map_err(|_| meshway_private_storage::Error::Io)?;
        directory.sync()?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(LIMIT + 1));
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| meshway_private_storage::Error::Io)?;
        if bytes.len() > LIMIT {
            return Err(Error::Record);
        }
        let envelope: Record<&RawValue> = decode(&bytes)?;
        if envelope.origin != origin {
            return Err(Error::Record);
        }
        let upgrading = match envelope.version {
            1 => true,
            VERSION => false,
            _ => return Err(Error::Record),
        };
        let record = if upgrading {
            // The old cache is inspected only for integrity and binding. It is
            // never deserialized as, or defaulted into, current authority.
            if let Some(saved) = envelope.saved {
                validate_obsolete(saved, envelope.binding.as_ref().ok_or(Error::Record)?)?;
            }
            Self {
                version: VERSION,
                origin: envelope.origin,
                keys: envelope.keys,
                enrollment: envelope.enrollment,
                binding: envelope.binding,
                saved: None,
                rotation: envelope.rotation,
                completed_rotation: envelope.completed_rotation,
            }
        } else {
            decode(&bytes)?
        };
        record.validate()?;
        if upgrading {
            archive(directory, &bytes)?;
            let mut buffer = Buffer(Zeroizing::new(Vec::with_capacity(LIMIT)));
            serde_json::to_writer(&mut buffer, &record).map_err(|_| Error::Record)?;
            // The lock remains held and both writes are synchronous and durable.
            // A failed archive never replaces the original identity record.
            directory.replace(RECORD, &buffer.0)?;
        }
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
            (None, None, None) if record.saved.is_none() => {}
            (Some(keys), Some(pending), None) if record.saved.is_none() => {
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
            (Some(keys), None, Some(binding)) if keys.matches(binding)? => {
                if let Some(saved) = &record.saved {
                    super::execution::validate_target(saved, binding).map_err(|_| Error::Record)?;
                }
            }
            _ => return Err(Error::Record),
        }
        Ok(())
    }
}

fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(meshway_serde::MapOnly(&mut decoder)).map_err(|_| Error::Record)?;
    decoder.end().map_err(|_| Error::Record)?;
    Ok(value)
}

/// Only record v1 may contain this opaque derived body. MapOnly recursively
/// rejects duplicate keys, including inside fields that are not execution inputs
/// anymore. Header values retain their original strong types.
fn validate_obsolete(raw: &RawValue, binding: &EnrollmentResponse) -> Result<(), Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Slice<'a> {
        digest: meshway_model::Digest,
        #[serde(borrow)]
        configuration: &'a RawValue,
    }
    let slice: Slice<'_> = decode(raw.get().as_bytes())?;
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

fn archive(directory: &meshway_private_storage::Directory, bytes: &[u8]) -> Result<(), Error> {
    let digest = meshway_model::Digest::from_bytes(Sha256::digest(bytes).into());
    let name = format!("control-v1-{digest}.json");
    if let Some(file) = directory.file(&name, false)? {
        let mut existing = Zeroizing::new(Vec::new());
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut existing)
            .map_err(|_| meshway_private_storage::Error::Io)?;
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

impl Control {
    pub(super) async fn persist(&mut self) -> Result<(), Error> {
        self.persist_record(false).await
    }

    /// Only the validated configuration-query transaction may preserve lineage.
    pub(super) async fn persist_target(&mut self) -> Result<(), Error> {
        self.persist_record(true).await
    }

    async fn persist_record(&mut self, target: bool) -> Result<(), Error> {
        self.ready()?;
        // Mutations happen under the single driver. Until fsync is acknowledged,
        // neither this driver nor a cancelled caller may use the new identity.
        self.dirty = true;
        if target {
            self.invalidate_guards();
        } else {
            self.invalidate_status_context();
            self.invalidate_snapshots();
        }
        let mut buffer = Buffer(Zeroizing::new(Vec::with_capacity(LIMIT)));
        serde_json::to_writer(&mut buffer, &self.record).map_err(|_| Error::Record)?;
        let directory = Arc::clone(&self.directory);
        self.write = Some(tokio::task::spawn_blocking(move || {
            ensure_marker(&directory).map_err(|_| meshway_private_storage::Error::UnsafeStorage)?;
            directory.replace(RECORD, &buffer.0)
        }));
        let result = self
            .write
            .as_mut()
            .ok_or(Error::ReloadRequired)?
            .await
            .map_err(|_| Error::ReloadRequired)?;
        self.write.take();
        result?;
        self.dirty = false;
        Ok(())
    }
}
