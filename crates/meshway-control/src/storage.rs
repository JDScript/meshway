//! Private Node control storage.
//!
//! `control.json` (record v5) holds only the identity envelope: keys, pending
//! enrollment, binding and rotation. Derived caches live in their own files
//! with their own format identifiers, so compiler changes never touch the
//! identity version: `netmap.json` (see `netmap::store`). A leftover
//! publication-era `execution.json` is neither read nor removed.
use crate::{
    Control, Error,
    identity::{self, Keys},
};
use meshway_protocol::{EnrollmentIntent, EnrollmentResponse, WireSecret};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    sync::Arc,
};
use zeroize::Zeroizing;

pub(super) const LIMIT: usize = 16 * 1024 * 1024;
const RECORD: &str = "control.json";
const MARKER: &[u8] = b"meshway-control/1\n";
const VERSION: u8 = 5;

const NO_LOCK: &str = "node-control.lock is missing";

fn marker(directory: &meshway_private_storage::Directory) -> Result<bool, Error> {
    let file = directory
        .file("node-control.lock", false)?
        .ok_or(Error::Record(NO_LOCK))?;
    let mut bytes = Vec::new();
    file.take((MARKER.len() + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| meshway_private_storage::Error::Io)?;
    if bytes.is_empty() {
        Ok(false)
    } else if bytes == MARKER {
        Ok(true)
    } else {
        Err(Error::Record(
            "node-control.lock does not hold the Meshway control marker",
        ))
    }
}
pub(super) fn ensure_marker(directory: &meshway_private_storage::Directory) -> Result<(), Error> {
    let mut file = directory
        .file("node-control.lock", false)?
        .ok_or(Error::Record(NO_LOCK))?;
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
    /// The operator's durable leave intent, saved before the request is sent.
    /// Only the Server's confirmation or its rejection of this identity ends
    /// it locally (`forget`); a definitive refusal withdraws it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leave: Option<meshway_model::RequestId>,
    /// A definitive identity rejection forbids offline cache activation until
    /// a fresh signed sync succeeds. This is local identity lifecycle state.
    #[serde(default)]
    pub cache_blocked: bool,
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
        return Err(Error::Record("a private file exceeds the 16 MiB limit"));
    }
    Ok(Some(bytes))
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut buffer = Buffer(Zeroizing::new(Vec::new()));
    serde_json::to_writer(&mut buffer, value)
        .map_err(|_| Error::Record("a private record exceeds the 16 MiB limit"))?;
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
                return Err(Error::Record(
                    "control.json is missing from an initialized identity directory",
                ));
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
                    return Err(Error::Record(
                        "the identity directory holds other files but no control.json",
                    ));
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
                leave: None,
                cache_blocked: false,
            });
        };
        file.sync_all()
            .map_err(|_| meshway_private_storage::Error::Io)?;
        directory.sync()?;
        let bytes = read(directory, RECORD)?
            .ok_or(Error::Record("control.json disappeared while opening"))?;
        let mut record: Self = decode(&bytes)
            .map_err(|_| Error::Record("control.json is not a valid identity record"))?;
        if record.version != VERSION {
            return Err(Error::Record(
                "control.json has an unsupported record version",
            ));
        }
        record.validate()?;
        if record.origin != origin {
            record.rebind(origin)?;
        }
        ensure_marker(directory)?;
        Ok(record)
    }

    /// A record without identity or pending work (a Node that left, or
    /// cancelled its enrollment) binds to no Server: it takes the configured
    /// origin. Anything else still belongs to its own Server.
    fn rebind(&mut self, configured: &str) -> Result<(), Error> {
        let held = if self.leave.is_some() {
            Some(crate::Held::Leaving)
        } else if self.binding.is_some() {
            Some(crate::Held::Identity)
        } else if self.keys.is_some() || self.enrollment.is_some() {
            Some(crate::Held::Enrollment)
        } else {
            None
        };
        if let Some(held) = held {
            return Err(Error::Origin {
                held,
                origin: self.origin.clone(),
                configured: configured.into(),
            });
        }
        tracing::info!(
            previous = %self.origin,
            configured,
            "Identity record holds no identity; it binds to the configured Server"
        );
        configured.clone_into(&mut self.origin);
        Ok(())
    }

    /// The same origin with no identity: the record of a Node that left.
    pub(super) fn unenrolled(&self) -> Self {
        Self {
            version: VERSION,
            origin: self.origin.clone(),
            keys: None,
            enrollment: None,
            binding: None,
            rotation: None,
            completed_rotation: None,
            leave: None,
            cache_blocked: false,
        }
    }

    fn validate(&self) -> Result<(), Error> {
        let record = self;
        if record.completed_rotation.is_some() && record.binding.is_none() {
            return Err(Error::Record(
                "control.json records a completed key rotation without an identity",
            ));
        }
        if record.leave.is_some()
            && (record.binding.is_none()
                || record.rotation.is_some()
                || record.enrollment.is_some())
        {
            return Err(Error::Record(
                "control.json records a leave without an identity, or beside pending work",
            ));
        }
        if let Some(rotation) = &record.rotation {
            rotation.validate(record.binding.as_ref().ok_or(Error::Record(
                "control.json records a key rotation without an identity",
            ))?)?;
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
                    return Err(Error::Record(
                        "control.json's pending enrollment does not match its keys",
                    ));
                }
            }
            (Some(keys), None, Some(binding)) if keys.matches(binding)? => {}
            // Re-enrollment of the held identity with its own keys.
            (Some(keys), Some(pending), Some(binding))
                if keys.matches(binding)?
                    && record.rotation.is_none()
                    && pending.access_key.is_none()
                    && pending.intent.control_key == keys.control()?
                    && pending.intent.dataplane_key == keys.dataplane()?
                    && pending.intent.code_challenge == identity::challenge(&pending.verifier)
                    && pending.code.is_some() == pending.approved_network.is_some()
                    && pending.intent.existing.as_ref().is_some_and(|existing| {
                        existing.network_id == binding.network_id
                            && existing.node_id == binding.node_id
                            && existing.key_id == binding.keys.control.id
                            && existing.key_generation == binding.keys.control.generation
                    }) => {}
            _ => {
                return Err(Error::Record(
                    "control.json's keys, pending enrollment and identity do not match",
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    const INVALID: Error = Error::Record("a private record does not decode");
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(meshway_model::wire::MapOnly(&mut decoder)).map_err(|_| INVALID)?;
    decoder.end().map_err(|_| INVALID)?;
    Ok(value)
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
    /// Fences immediately, then durably records a definitive identity
    /// rejection before exposing its result. No asynchronous write can race
    /// this exclusive owner; an uncertain outcome requires reopening.
    pub(crate) fn block_netmap_cache(&mut self) -> Result<(), Error> {
        self.revoke_netmap();
        if self.record.cache_blocked {
            return Ok(());
        }
        self.record.cache_blocked = true;
        let bytes = encode(&self.record)?;
        self.dirty = true;
        self.directory.replace(RECORD, &bytes)?;
        self.dirty = false;
        Ok(())
    }

    /// Identity envelope write. Withdraws the published lease first.
    pub(super) async fn persist(&mut self) -> Result<(), Error> {
        self.ready()?;
        self.invalidate_status_context();
        self.revoke_netmap();
        let bytes = encode(&self.record)?;
        self.write_private(RECORD, bytes).await
    }

    /// Pinned Server key replacement. The identity is unchanged, so neither
    /// the published lease nor the status context ends.
    pub(super) async fn persist_binding(&mut self) -> Result<(), Error> {
        let bytes = encode(&self.record)?;
        self.write_private(RECORD, bytes).await
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
        // A cancelled caller leaves the outcome unknown: `dirty` then keeps
        // every netmap, State and receipt output withheld until reopening.
        self.dirty = true;
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
            self.revoke_netmap();
            return Err(error.into());
        }
        self.dirty = false;
        Ok(())
    }
}
