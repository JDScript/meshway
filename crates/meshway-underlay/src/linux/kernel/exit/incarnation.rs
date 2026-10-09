use serde::{Deserialize, Serialize};

/// Exact producer-issued opaque handle. Every byte participates in identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExitIncarnation([u8; 32]);

impl ExitIncarnation {
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
    pub(super) fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }
}
impl From<[u8; 32]> for ExitIncarnation {
    fn from(bytes: [u8; 32]) -> Self {
        Self::new(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_256_bits_round_trip_and_distinguish_projection_identity() {
        let original = ExitIncarnation::new([7; 32]);
        let mut entries = std::collections::BTreeSet::from([original]);
        for index in 0..32 {
            let mut bytes = original.bytes();
            bytes[index] ^= 1;
            entries.insert(bytes.into());
        }
        assert_eq!(entries.len(), 33);
        for entry in entries {
            let wire = serde_json::to_vec(&entry).unwrap();
            assert_eq!(
                serde_json::from_slice::<ExitIncarnation>(&wire).unwrap(),
                entry
            );
        }
    }
}
