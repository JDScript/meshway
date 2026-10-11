use meshway_model::Digest;
use serde::Serialize;
use sha2_fast::{Digest as _, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("configuration serialization failed")]
pub struct SerializationError;

/// Feeds the serializer straight into the hash: no serialized copy (and so
/// no serialized secret) outlives the call, and whole netmaps of a large
/// network hash with the CPU's SHA-256 instructions where it has them.
struct Hashing(Sha256);

impl std::io::Write for Hashing {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Hash the canonical typed serialization: stable struct order and sorted maps.
pub fn digest<T: Serialize>(value: &T) -> Result<Digest, SerializationError> {
    let mut hashing = Hashing(Sha256::new());
    serde_json::to_writer(&mut hashing, value).map_err(|_| SerializationError)?;
    Ok(Digest::from_bytes(hashing.0.finalize().into()))
}
