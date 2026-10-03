//! UTC RFC 3339 on the wire, exact Unix milliseconds in execution and signatures.
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

fn date(value: u64) -> Result<DateTime<Utc>, &'static str> {
    i64::try_from(value)
        .ok()
        .and_then(DateTime::from_timestamp_millis)
        .filter(|date| date.year() <= 9999)
        .ok_or("timestamp is outside the RFC 3339 range")
}

fn millis(date: DateTime<Utc>) -> Result<u64, &'static str> {
    if date.year() > 9999 || !date.timestamp_subsec_nanos().is_multiple_of(1_000_000) {
        return Err("timestamp requires exact milliseconds in the RFC 3339 range");
    }
    u64::try_from(date.timestamp_millis()).map_err(|_| "timestamp precedes Unix epoch")
}

pub fn serialize<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    date(*value)
        .map_err(serde::ser::Error::custom)?
        .serialize(serializer)
}

pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    millis(DateTime::<Utc>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
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
        Option::<DateTime<Utc>>::deserialize(deserializer)?
            .map(millis)
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}
