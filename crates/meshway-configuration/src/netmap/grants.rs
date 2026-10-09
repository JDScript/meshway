//! Grants are keyed by a structured key, so the wire carries them as an array
//! in strictly increasing key order: one canonical encoding, no duplicates.
use super::{Grant, GrantKey};
use serde::{Deserialize, Serializer, ser::SerializeSeq};
use std::collections::BTreeMap;

pub fn serialize<S: Serializer>(
    grants: &BTreeMap<GrantKey, Grant>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let mut seq = serializer.serialize_seq(Some(grants.len()))?;
    for grant in grants.values() {
        seq.serialize_element(grant)?;
    }
    seq.end()
}

pub fn deserialize<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<GrantKey, Grant>, D::Error> {
    let list = Vec::<Grant>::deserialize(d)?;
    let mut grants = BTreeMap::new();
    let mut previous: Option<GrantKey> = None;
    for grant in list {
        let key = grant.key();
        if previous.is_some_and(|previous| previous >= key) {
            return Err(serde::de::Error::custom(
                "grants must be unique and in increasing key order",
            ));
        }
        previous = Some(key);
        grants.insert(key, grant);
    }
    Ok(grants)
}
