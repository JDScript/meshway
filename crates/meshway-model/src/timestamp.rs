//! UTC RFC 3339 on the wire, exact Unix milliseconds in execution and signatures.
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

/// Schema-only marker for the string representation used by this module.
/// Execution and signature values remain exact Unix milliseconds (`u64`).
pub struct Timestamp;

impl utoipa::PartialSchema for Timestamp {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{ObjectBuilder, SchemaFormat, Type};
        ObjectBuilder::new()
            .schema_type(Type::String)
            .format(Some(SchemaFormat::Custom("meshway-timestamp-ms".into())))
            .description(Some("Nonnegative Unix time as an RFC 3339 string with exact millisecond precision and UTC year at most 9999. Offsets and lowercase t/z are accepted; fractional digits after milliseconds must all be zero. Leap seconds are rejected because they have no lossless Unix-millisecond representation."))
            .into()
    }
}
impl utoipa::ToSchema for Timestamp {}

fn date(value: u64) -> Result<DateTime<Utc>, &'static str> {
    i64::try_from(value)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .filter(|date| date.year() <= 9999)
        .ok_or("timestamp is outside the RFC 3339 range")
}

fn millis(date: DateTime<Utc>) -> Result<u64, &'static str> {
    if date.year() > 9999
        || date.timestamp_subsec_nanos() >= 1_000_000_000
        || !date.timestamp_subsec_nanos().is_multiple_of(1_000_000)
    {
        return Err("timestamp requires exact milliseconds in the RFC 3339 range");
    }
    u64::try_from(date.timestamp_millis()).map_err(|_| "timestamp precedes Unix epoch")
}

fn parse(value: &str) -> Result<u64, &'static str> {
    const INVALID: &str =
        "expected nonnegative RFC 3339 time with exact milliseconds and no leap second";
    let bytes = value.as_bytes();
    // Check original text before Chrono can truncate fractions beyond nanoseconds
    // or normalize a leap second to a different Unix instant.
    if bytes.len() < 20
        || !bytes[..19]
            .iter()
            .enumerate()
            .all(|(index, byte)| match index {
                4 | 7 => *byte == b'-',
                10 => matches!(*byte, b'T' | b't'),
                13 | 16 => *byte == b':',
                _ => byte.is_ascii_digit(),
            })
        || bytes[17] > b'5'
    {
        return Err(INVALID);
    }
    let mut timezone = 19;
    if bytes[timezone] == b'.' {
        let start = timezone + 1;
        timezone = start;
        while bytes.get(timezone).is_some_and(u8::is_ascii_digit) {
            if timezone >= start + 3 && bytes[timezone] != b'0' {
                return Err(INVALID);
            }
            timezone += 1;
        }
        if timezone == start {
            return Err(INVALID);
        }
    }
    match &bytes[timezone..] {
        [b'Z' | b'z'] => {}
        [b'+' | b'-', a, b, b':', c, d]
            if [a, b, c, d].iter().all(|byte| byte.is_ascii_digit()) => {}
        _ => return Err(INVALID),
    }
    let date = DateTime::parse_from_rfc3339(value).map_err(|_| INVALID)?;
    millis(date.with_timezone(&Utc))
}

pub fn serialize<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    date(*value)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

pub mod optional {
    use super::*;

    pub fn serialize<S: serde::Serializer>(
        value: &Option<u64>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(date)
            .transpose()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .as_deref()
            .map(parse)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
