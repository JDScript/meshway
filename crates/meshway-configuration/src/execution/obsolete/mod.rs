//! Integrity checks for historical demand-1/demand-2 slice bodies.
use super::{
    DemandDecision, DiagnosticPermit, DialCandidate, PathPermit, PeerConfiguration,
    TrustedDnsAssociation, TrustedNode,
};
use crate::{DnsConfiguration, NodeAddresses, NodeIdentity, transport::PreparedListener};
use meshway_model::{Digest, NetworkId, NodeId, ObservationId, PublicationId, RequestId, Revision};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

/// Format failures, separate from the caller's publication or enrollment binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObsoleteSliceError {
    #[error("invalid obsolete execution JSON or header")]
    Decode,
    #[error("invalid obsolete execution field set")]
    Fields,
    #[error("invalid obsolete execution field")]
    Field,
    #[error("invalid obsolete execution map")]
    Map,
    #[error("invalid obsolete execution DNS field set")]
    Dns,
    #[error("obsolete execution raw digest mismatch")]
    DigestMismatch,
}

/// Strongly typed historical metadata for caller-owned context checks.
/// This is evidence only, never a current execution target or authority.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsoleteSliceHeader {
    pub network_id: NetworkId,
    pub node_id: NodeId,
    pub revision: Revision,
    pub publication: PublicationId,
    pub dependency_digest: Digest,
    pub self_keys: TrustedNode,
}

/// Validate the exact historical field shape, typed values and raw-byte digest.
/// Duplicate keys are rejected recursively, including in opaque fields; unknown
/// fields retain the historical types' deserialization rules. Callers must check
/// the returned header against their own enrollment/publication context and must
/// never deserialize this body into current execution authority.
///
/// Unlike [`crate::digest`], which hashes canonical serialization of a typed
/// value, this hashes the original JSON bytes, preserving whitespace, key order
/// and omitted defaults. It does not canonicalize or reconstruct an old target.
pub fn validate_obsolete_slice(
    configuration: &RawValue,
    digest: Digest,
) -> Result<ObsoleteSliceHeader, ObsoleteSliceError> {
    let bytes = configuration.get().as_bytes();
    // Ordinary IgnoredAny would hide duplicates in otherwise opaque fields.
    let _: serde::de::IgnoredAny = decode(bytes)?;
    let fields: BTreeMap<String, &RawValue> = decode(bytes)?;
    const FIELDS: &[&str] = &[
        "networkId",
        "nodeId",
        "revision",
        "publication",
        "dependencyDigest",
        "enabled",
        "hostname",
        "identity",
        "addresses",
        "selfKeys",
        "listeners",
        "dialers",
        "peers",
        "permits",
        "diagnostics",
        "decisions",
        "dns",
        "exitEnabled",
    ];
    if FIELDS.iter().any(|key| !fields.contains_key(*key))
        || fields.keys().any(|key| !FIELDS.contains(&key.as_str()))
    {
        return Err(ObsoleteSliceError::Fields);
    }
    validate_old_fields(&fields)?;
    let header = decode(bytes)?;
    if digest.as_bytes() != Sha256::digest(bytes).as_slice() {
        return Err(ObsoleteSliceError::DigestMismatch);
    }
    Ok(header)
}

fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, ObsoleteSliceError> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(meshway_serde::MapOnly(&mut decoder))
        .map_err(|_| ObsoleteSliceError::Decode)?;
    decoder.end().map_err(|_| ObsoleteSliceError::Decode)?;
    Ok(value)
}

fn validate_old_fields(fields: &BTreeMap<String, &RawValue>) -> Result<(), ObsoleteSliceError> {
    fn field<T: DeserializeOwned>(
        fields: &BTreeMap<String, &RawValue>,
        key: &str,
    ) -> Result<(), ObsoleteSliceError> {
        serde_json::from_str::<T>(fields.get(key).ok_or(ObsoleteSliceError::Field)?.get())
            .map(|_| ())
            .map_err(|_| ObsoleteSliceError::Field)
    }
    fn map<K: DeserializeOwned + Ord, V: DeserializeOwned>(
        fields: &BTreeMap<String, &RawValue>,
        key: &str,
    ) -> Result<(), ObsoleteSliceError> {
        let mut decoder = serde_json::Deserializer::from_str(
            fields.get(key).ok_or(ObsoleteSliceError::Field)?.get(),
        );
        let _: BTreeMap<K, V> =
            meshway_serde::unique_map(&mut decoder).map_err(|_| ObsoleteSliceError::Map)?;
        decoder.end().map_err(|_| ObsoleteSliceError::Map)
    }
    field::<bool>(fields, "enabled")?;
    field::<String>(fields, "hostname")?;
    field::<NodeIdentity>(fields, "identity")?;
    field::<NodeAddresses>(fields, "addresses")?;
    map::<String, PreparedListener>(fields, "listeners")?;
    map::<String, DialCandidate>(fields, "dialers")?;
    map::<NodeId, PeerConfiguration>(fields, "peers")?;
    map::<RequestId, PathPermit>(fields, "permits")?;
    map::<ObservationId, DiagnosticPermit>(fields, "diagnostics")?;
    map::<RequestId, DemandDecision>(fields, "decisions")?;
    field::<bool>(fields, "exitEnabled")?;
    let dns: BTreeMap<String, &RawValue> = decode(
        fields
            .get("dns")
            .ok_or(ObsoleteSliceError::Dns)?
            .get()
            .as_bytes(),
    )?;
    if dns.len() != 4 {
        return Err(ObsoleteSliceError::Dns);
    }
    field::<DnsConfiguration>(&dns, "settings")?;
    map::<String, NodeAddresses>(&dns, "meshHosts")?;
    field::<Vec<TrustedDnsAssociation>>(&dns, "associations")?;
    field::<String>(&dns, "profile")
}

#[cfg(test)]
mod tests;
