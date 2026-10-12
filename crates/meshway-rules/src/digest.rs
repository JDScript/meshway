use sha2::{Digest as _, Sha256};
use std::{fmt, str::FromStr};

/// `sha256:<hex>` of a complete artifact. It is the only content identity:
/// netmap references, catalog entries and cache file names all use it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("A digest is `sha256:` followed by 64 lowercase hex digits.")]
pub struct InvalidDigest;

const PREFIX: &str = "sha256:";

impl Digest {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn hex(&self) -> String {
        hex(&self.0)
    }
    /// Content-addressed file name, `sha256-<hex>.mwrs`, used by the catalog
    /// `artifacts/` directory and by private caches.
    pub fn file_name(&self) -> String {
        format!("sha256-{}.mwrs", self.hex())
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

/// Lowercase hex only, so every value has exactly one spelling.
pub(crate) fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    let text = text.as_bytes();
    if text.len() != N * 2 {
        return None;
    }
    let mut out = [0; N];
    for (i, pair) in text.as_chunks::<2>().0.iter().enumerate() {
        out[i] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Some(out)
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PREFIX}{}", self.hex())
    }
}
impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl FromStr for Digest {
    type Err = InvalidDigest;
    fn from_str(text: &str) -> Result<Self, InvalidDigest> {
        text.strip_prefix(PREFIX)
            .and_then(unhex::<32>)
            .map(Self)
            .ok_or(InvalidDigest)
    }
}
impl serde::Serialize for Digest {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> serde::Deserialize<'de> for Digest {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn one_spelling() {
        let d = Digest::of(b"");
        let text = d.to_string();
        assert_eq!(
            text,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(text.parse::<Digest>(), Ok(d));
        assert!(text.to_uppercase().parse::<Digest>().is_err());
        assert!(text.replace("sha256:", "").parse::<Digest>().is_err());
        assert!(format!("{text}0").parse::<Digest>().is_err());
        assert_eq!(
            d.file_name(),
            "sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855.mwrs"
        );
    }
}
