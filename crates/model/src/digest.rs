use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

/// A lossless 32-byte digest represented as exactly 64 lowercase hex digits.
///
/// This value does not compute hashes or define their input encoding or purpose.
/// In particular, serializability does not authorize publication of secret hashes.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, ToSchema)]
#[schema(value_type = String, min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
pub struct Digest([u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("expected exactly 64 lowercase hexadecimal digits")]
pub struct InvalidDigest;

impl Digest {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn into_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl From<[u8; 32]> for Digest {
    fn from(value: [u8; 32]) -> Self {
        Self::from_bytes(value)
    }
}

impl FromStr for Digest {
    type Err = InvalidDigest;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        fn nibble(byte: u8) -> Result<u8, InvalidDigest> {
            match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                _ => Err(InvalidDigest),
            }
        }

        if value.len() != 64 {
            return Err(InvalidDigest);
        }
        let mut bytes = [0; 32];
        for (output, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
            *output = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
