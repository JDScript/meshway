use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::ValidationError;

/// A nonzero TCP/UDP port. Successful validation says nothing about availability.
#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema,
)]
#[serde(try_from = "u16", into = "u16")]
#[schema(value_type = u16, minimum = 1, maximum = 65535)]
pub struct Port(u16);

impl Port {
    pub const fn new(value: u16) -> Result<Self, ValidationError> {
        if value == 0 {
            Err(ValidationError::Port)
        } else {
            Ok(Self(value))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for Port {
    type Error = ValidationError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Port> for u16 {
    fn from(value: Port) -> Self {
        value.get()
    }
}
