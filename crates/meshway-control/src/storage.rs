use crate::{
    Control, Error,
    identity::{self, Keys},
};
use meshway_configuration::execution::CompiledNode;
use meshway_protocol::{EnrollmentIntent, EnrollmentResponse, WireSecret};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    sync::Arc,
};
use zeroize::Zeroizing;

pub(super) const LIMIT: usize = 16 * 1024 * 1024;
const RECORD: &str = "control.json";

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
    #[serde(deserialize_with = "Option::deserialize")]
    pub saved: Option<CompiledNode>,
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
}

impl Record {
    pub fn load(
        directory: &meshway_private_storage::Directory,
        origin: &str,
    ) -> Result<Self, Error> {
        let Some(file) = directory.file(RECORD, false)? else {
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
                version: 1,
                origin: origin.into(),
                keys: None,
                enrollment: None,
                binding: None,
                saved: None,
            });
        };
        let mut bytes = Zeroizing::new(Vec::with_capacity(LIMIT + 1));
        file.take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| meshway_private_storage::Error::Io)?;
        if bytes.len() > LIMIT {
            return Err(Error::Record);
        }
        let mut decoder = serde_json::Deserializer::from_slice(&bytes);
        let record =
            Self::deserialize(meshway_serde::MapOnly(&mut decoder)).map_err(|_| Error::Record)?;
        decoder.end().map_err(|_| Error::Record)?;
        if record.version != 1 || record.origin != origin {
            return Err(Error::Record);
        }
        match (&record.keys, &record.enrollment, &record.binding) {
            (None, None, None) if record.saved.is_none() => {}
            (Some(keys), Some(pending), None) if record.saved.is_none() => {
                if pending.intent.existing.is_some()
                    || pending.intent.control_key != keys.control()?
                    || pending.intent.dataplane_key != keys.dataplane()?
                    || pending.intent.code_challenge != identity::challenge(&pending.verifier)
                    || pending.code.is_some() != pending.approved_network.is_some()
                {
                    return Err(Error::Record);
                }
            }
            (Some(keys), None, Some(binding)) if keys.matches(binding)? => {
                if let Some(saved) = &record.saved {
                    super::execution::validate_target(saved, binding)?;
                }
            }
            _ => return Err(Error::Record),
        }
        Ok(record)
    }
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
        self.ready()?;
        // Mutations happen under the single driver. Until fsync is acknowledged,
        // neither this driver nor a cancelled caller may use the new identity.
        self.dirty = true;
        let mut buffer = Buffer(Zeroizing::new(Vec::with_capacity(LIMIT)));
        serde_json::to_writer(&mut buffer, &self.record).map_err(|_| Error::Record)?;
        let directory = Arc::clone(&self.directory);
        self.write = Some(tokio::task::spawn_blocking(move || {
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
