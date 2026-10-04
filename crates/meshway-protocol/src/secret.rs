use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;
use zeroize::Zeroizing;

/// Secret wire string. Serialization and borrowing are explicit; Debug, Display,
/// and Clone are deliberately unavailable. The owned buffer is wiped on drop.
#[derive(ToSchema)]
#[schema(value_type = String)]
pub struct WireSecret(Zeroizing<String>);

impl WireSecret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl Serialize for WireSecret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}

impl<'de> Deserialize<'de> for WireSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}
