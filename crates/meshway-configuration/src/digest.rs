use meshway_model::Digest;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("configuration serialization failed")]
pub struct SerializationError;

/// Hash the canonical typed serialization: stable struct order and sorted maps.
/// Temporary serialized secrets are zeroized after hashing.
pub fn digest<T: Serialize>(value: &T) -> Result<Digest, SerializationError> {
    let bytes = zeroize::Zeroizing::new(serde_json::to_vec(value).map_err(|_| SerializationError)?);
    Ok(Digest::from_bytes(Sha256::digest(bytes.as_slice()).into()))
}
