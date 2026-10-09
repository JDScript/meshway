use super::SystemDnsError;
use meshway_private_storage::Directory;
use serde::{Serialize, de::DeserializeOwned};
use std::{io::Read, sync::Arc};

const LIMIT: usize = 256 * 1024;
pub(super) fn read<T: DeserializeOwned>(
    directory: &Directory,
    name: &str,
) -> Result<Option<T>, SystemDnsError> {
    let Some(file) = directory
        .file(name, false)
        .map_err(|_| SystemDnsError::Storage)?
    else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SystemDnsError::Storage)?;
    if bytes.len() > LIMIT {
        return Err(SystemDnsError::Capacity);
    }
    super::super::decode_optional_record(&bytes)
}
pub(super) fn save<T: Serialize>(
    directory: &Directory,
    name: &str,
    value: &T,
) -> Result<(), SystemDnsError> {
    let bytes = serde_json::to_vec(value).map_err(|_| SystemDnsError::Storage)?;
    if bytes.len() > LIMIT {
        return Err(SystemDnsError::Capacity);
    }
    directory
        .replace(name, &bytes)
        .map_err(|_| SystemDnsError::Storage)
}
pub(super) struct Writes {
    writes: meshway_private_storage::Writes<SystemDnsError>,
}
impl Default for Writes {
    fn default() -> Self {
        Self {
            writes: meshway_private_storage::Writes::poisoning(|| SystemDnsError::Storage),
        }
    }
}
impl Writes {
    pub async fn settle(&mut self) -> Result<(), SystemDnsError> {
        self.writes.settle().await
    }
    pub async fn run(
        &mut self,
        work: impl FnOnce() -> Result<(), SystemDnsError> + Send + 'static,
    ) -> Result<(), SystemDnsError> {
        self.writes.run(work).await
    }
    pub async fn save<T: Serialize>(
        &mut self,
        directory: Arc<Directory>,
        name: &'static str,
        value: &T,
    ) -> Result<(), SystemDnsError> {
        let bytes = serde_json::to_vec(value).map_err(|_| SystemDnsError::Storage)?;
        if bytes.len() > LIMIT {
            return Err(SystemDnsError::Capacity);
        }
        self.run(move || {
            directory
                .replace(name, &bytes)
                .map_err(|_| SystemDnsError::Storage)
        })
        .await
    }
}
