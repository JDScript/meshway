use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize, Serializer};
use utoipa::{
    PartialSchema, ToSchema,
    openapi::schema::{ObjectBuilder, Type},
};

use crate::{ValidationError, schema::positive_decimal_pattern};

const UNITS: [(&str, u64); 4] = [("ms", 1), ("s", 1_000), ("m", 60_000), ("h", 3_600_000)];

/// Positive integer followed by `ms`, `s`, `m`, or `h`.
///
/// Engineering boundary: canonical ASCII decimal (no sign, leading zero,
/// fraction, or whitespace), with a total of 1..=u64::MAX milliseconds.
/// The chosen unit is preserved on serialization; `60s` is not rewritten as
/// `1m`. This is representability, not an approved timeout/resource budget.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(try_from = "String")]
pub struct PositiveDuration {
    source: String,
    milliseconds: u64,
}

impl PositiveDuration {
    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub const fn as_millis(&self) -> u64 {
        self.milliseconds
    }

    pub const fn as_std(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.milliseconds)
    }
}

impl TryFrom<String> for PositiveDuration {
    type Error = ValidationError;

    fn try_from(source: String) -> Result<Self, Self::Error> {
        let (digits, multiplier) = UNITS
            .iter()
            .find_map(|(unit, multiplier)| {
                source.strip_suffix(unit).map(|digits| (digits, multiplier))
            })
            .ok_or(ValidationError::DurationSyntax)?;
        if !matches!(digits.as_bytes().first(), Some(b'1'..=b'9'))
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ValidationError::DurationSyntax);
        }
        let milliseconds = digits
            .parse::<u64>()
            .ok()
            .and_then(|value| value.checked_mul(*multiplier))
            .ok_or(ValidationError::DurationRange)?;
        Ok(Self {
            source,
            milliseconds,
        })
    }
}

impl FromStr for PositiveDuration {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl fmt::Display for PositiveDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for PositiveDuration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl PartialSchema for PositiveDuration {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        let alternatives: Vec<_> = UNITS
            .iter()
            .map(|(unit, multiplier)| {
                format!("{}{unit}", positive_decimal_pattern(u64::MAX / multiplier))
            })
            .collect();
        ObjectBuilder::new()
            .schema_type(Type::String)
            // Unlike $, this end assertion also rejects a final newline in JS.
            .pattern(Some(format!("^(?:{})(?![\\s\\S])", alternatives.join("|"))))
            .description(Some("Positive canonical decimal with ms/s/m/h; total must fit u64 milliseconds. Unit spelling is preserved. Consumers enforce resource and lease limits separately."))
            .into()
    }
}

impl ToSchema for PositiveDuration {}
