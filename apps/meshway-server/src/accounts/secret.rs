use std::fmt::Write;

use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::Error;

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

pub(crate) fn random_uuid() -> Result<Uuid, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::ENTROPY)?;
    // RFC 9562 version 4: generated independently of all credential material.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

pub(crate) fn issue_token() -> Result<(Secret, [u8; 32]), Error> {
    let mut random = Zeroizing::new([0u8; 32]);
    getrandom::fill(random.as_mut()).map_err(|_| Error::ENTROPY)?;
    let mut encoded = Zeroizing::new(String::with_capacity(68));
    encoded.push_str("mws_");
    for byte in random.iter() {
        write!(encoded, "{byte:02x}").map_err(|_| Error::Internal("token formatting"))?;
    }
    let digest = token_digest(&encoded)?;
    Ok((Secret(encoded), digest))
}

pub(crate) fn token_digest(token: &str) -> Result<[u8; 32], Error> {
    if token.len() != 68
        || !token.starts_with("mws_")
        || !token.as_bytes()[4..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return Err(Error::InvalidSession);
    }
    let mut hash = Sha256::new();
    hash.update(b"meshway:user-session:sha256:v1\0");
    hash.update(token.as_bytes());
    Ok(hash.finalize().into())
}

/// A separate token namespace for each one-time credential purpose.
pub(crate) fn issue_one_time(prefix: &str) -> Result<(Secret, [u8; 32]), Error> {
    let (token, _) = issue_token()?;
    let token = Secret::new(format!("{prefix}{}", &token.expose()[4..]));
    let digest = one_time_digest(prefix, token.expose())?;
    Ok((token, digest))
}

pub(crate) fn one_time_digest(prefix: &str, token: &str) -> Result<[u8; 32], Error> {
    let value = token.strip_prefix(prefix).ok_or(Error::InvalidReset)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::InvalidReset);
    }
    Ok(Sha256::digest(token.as_bytes()).into())
}
