use meshway_protocol::WireSecret;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, sync::Arc};
use utoipa::ToSchema;

/// A bounded secret input shared through the local command owner. Cloning shares
/// the zeroizing allocation; diagnostics never reveal its value.
#[derive(Clone, ToSchema)]
#[schema(value_type = String, min_length = 1, max_length = 4096)]
pub struct EnrollmentKey(Arc<WireSecret>);

impl EnrollmentKey {
    pub fn new(value: String) -> Result<Self, &'static str> {
        if value.is_empty()
            || value.len() > 4096
            || !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err("Invalid enrollment key input.");
        }
        Ok(Self(Arc::new(WireSecret::new(value))))
    }
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}
impl fmt::Debug for EnrollmentKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnrollmentKey([REDACTED])")
    }
}
impl Serialize for EnrollmentKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}
impl<'de> Deserialize<'de> for EnrollmentKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_inputs_are_bounded_and_debug_is_redacted() {
        let key = EnrollmentKey::new("fixture-private-value".into()).unwrap();
        assert!(!format!("{key:?}").contains(key.expose()));
        assert_eq!(key.clone().expose(), key.expose());
        for value in [String::new(), "key with spaces".into(), "x".repeat(4097)] {
            assert!(EnrollmentKey::new(value).is_err());
        }
    }
}
