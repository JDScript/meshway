use std::fmt::Write;

use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::AccountError;

/// Owned secret storage, wiped on drop. No Debug/Display/Clone/serialization.
/// The caller must also protect any original input buffers it retains.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    /// Explicit borrowing for delivery or use as a credential.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

pub(crate) fn random_uuid() -> Result<Uuid, AccountError> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| AccountError::Unavailable)?;
    // RFC 9562 version 4: generated independently of all credential material.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

pub(crate) fn issue_token() -> Result<(Secret, [u8; 32]), AccountError> {
    let mut random = Zeroizing::new([0u8; 32]);
    getrandom::fill(random.as_mut()).map_err(|_| AccountError::Unavailable)?;
    let mut encoded = Zeroizing::new(String::with_capacity(68));
    encoded.push_str("mws_");
    for byte in random.iter() {
        write!(encoded, "{byte:02x}").map_err(|_| AccountError::Unavailable)?;
    }
    let digest = token_digest(&encoded)?;
    Ok((Secret(encoded), digest))
}

pub(crate) fn token_digest(token: &str) -> Result<[u8; 32], AccountError> {
    if token.len() != 68
        || !token.starts_with("mws_")
        || !token.as_bytes()[4..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(AccountError::InvalidSession);
    }
    let mut hash = Sha256::new();
    hash.update(b"meshway:user-session:sha256:v1\0");
    hash.update(token.as_bytes());
    Ok(hash.finalize().into())
}
