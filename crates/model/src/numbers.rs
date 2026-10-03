use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::{
        RefOr, Schema,
        schema::{ObjectBuilder, Type},
    },
};

/// JavaScript's `Number.MAX_SAFE_INTEGER`, the upper bound for safe integer arithmetic.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Construction and increment failures never wrap or saturate a counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InvalidNumber {
    #[error("expected an integer between {minimum} and {MAX_SAFE_INTEGER}")]
    OutOfRange { minimum: u64 },
    #[error("increment exceeds the maximum safe JSON integer {MAX_SAFE_INTEGER}")]
    Overflow,
}

macro_rules! number {
    ($name:ident, $minimum:literal, $description:literal) => {
        #[doc = $description]
        ///
        /// Serialized as a JSON integer, bounded by [`MAX_SAFE_INTEGER`].
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(u64);

        // Explicit builders preserve both bounds with the pinned Utoipa version.
        impl PartialSchema for $name {
            fn schema() -> RefOr<Schema> {
                ObjectBuilder::new()
                    .schema_type(Type::Integer)
                    .description(Some($description))
                    .minimum(Some($minimum))
                    .maximum(Some(MAX_SAFE_INTEGER))
                    .into()
            }
        }

        impl ToSchema for $name {}

        impl $name {
            pub const MIN: Self = Self($minimum);
            pub const MAX: Self = Self(MAX_SAFE_INTEGER);

            pub const fn new(value: u64) -> Result<Self, InvalidNumber> {
                if value < Self::MIN.0 || value > MAX_SAFE_INTEGER {
                    Err(InvalidNumber::OutOfRange { minimum: $minimum })
                } else {
                    Ok(Self(value))
                }
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            /// Return the successor without changing this value on overflow.
            pub const fn checked_next(self) -> Result<Self, InvalidNumber> {
                if self.0 == MAX_SAFE_INTEGER {
                    Err(InvalidNumber::Overflow)
                } else {
                    Ok(Self(self.0 + 1))
                }
            }
        }

        impl TryFrom<u64> for $name {
            type Error = InvalidNumber;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for u64 {
            fn from(value: $name) -> Self {
                value.get()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }
    };
}

number!(Revision, 1, "A positive configuration revision.");
number!(
    Version,
    1,
    "A positive record version, distinct from a configuration revision."
);
number!(Sequence, 0, "A nonnegative sequence number.");
