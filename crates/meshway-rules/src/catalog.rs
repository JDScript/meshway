//! Signed provider catalog: `index.json`, its detached `index.json.sig`, and
//! the per-set records it is assembled from. The publishing pipeline signs,
//! the Server verifies against pinned keys; both use these functions.
//!
//! Directory layout (`dist` branch of the provider repository):
//!
//! ```text
//! index.json  index.json.sig
//! artifacts/sha256-<hex>.mwrs        immutable artifacts
//! sets/<collection>/<name>.json      SetRecord with the full report
//! ```

use crate::{
    Conversion, Counts, Digest, MAJOR, MINOR, Report, RuleSet, Summary,
    convert::Format,
    digest::{hex, unhex},
};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use std::{collections::BTreeMap, fmt, str::FromStr};

pub const SCHEMA: u32 = 1;
pub const INDEX: &str = "index.json";
pub const SIGNATURE: &str = "index.json.sig";
pub const ARTIFACTS: &str = "artifacts";
pub const SETS: &str = "sets";
/// Signed bytes are this prefix followed by the exact `index.json` bytes, so
/// a catalog key can never sign anything else meaningfully.
const CONTEXT: &[u8] = b"meshway-rules catalog v1\0";

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("Invalid catalog document: {0}")]
    Document(String),
    #[error("Invalid signature file: {0}")]
    SignatureFile(String),
    #[error("The catalog is signed by a key that is not trusted.")]
    UntrustedKey,
    #[error("The catalog signature does not verify.")]
    BadSignature,
    #[error("Unsupported catalog schema {0}.")]
    Schema(u32),
    #[error("Invalid set name {0:?}.")]
    SetName(String),
    #[error("Set {name}: {problem}")]
    Set { name: String, problem: String },
    #[error("Upstream {repo}@{reference} appears with more than one commit.")]
    Upstream { repo: String, reference: String },
    #[error("The catalog has no sets.")]
    Empty,
}

/// `ed25519:<64 lowercase hex>`; the full key is its own ID.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("A public key is `ed25519:` followed by 64 lowercase hex digits.")]
pub struct InvalidKey;

impl PublicKey {
    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.0
    }
}
impl FromStr for PublicKey {
    type Err = InvalidKey;
    /// Accepts the bare hex form too, as given on a command line.
    fn from_str(text: &str) -> Result<Self, InvalidKey> {
        let bytes = unhex::<32>(text.strip_prefix("ed25519:").unwrap_or(text)).ok_or(InvalidKey)?;
        VerifyingKey::from_bytes(&bytes)
            .map(Self)
            .map_err(|_| InvalidKey)
    }
}
impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ed25519:{}", hex(self.0.as_bytes()))
    }
}
impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Signing key from its 32-byte seed as 64 lowercase hex digits
/// (`openssl rand -hex 32`); surrounding whitespace is ignored.
pub fn signing_key(seed: &str) -> Result<SigningKey, InvalidKey> {
    unhex::<32>(seed.trim())
        .map(|s| SigningKey::from_bytes(&s))
        .ok_or(InvalidKey)
}

pub fn public_key(key: &SigningKey) -> PublicKey {
    PublicKey(key.verifying_key())
}

/// `index.json.sig`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Signature {
    pub key_id: String,
    /// 64-byte ed25519 signature, lowercase hex.
    pub signature: String,
}

pub fn sign(index: &[u8], key: &SigningKey) -> Signature {
    let message = [CONTEXT, index].concat();
    Signature {
        key_id: public_key(key).to_string(),
        signature: hex(&key.sign(&message).to_bytes()),
    }
}

/// Verifies `index.json.sig` against the trusted keys, then parses the
/// catalog. Sequence monotonicity is the caller's state and is checked there.
pub fn verify(
    index: &[u8],
    signature: &[u8],
    trusted: &[PublicKey],
) -> Result<Catalog, CatalogError> {
    let signature: Signature = serde_json::from_slice(signature)
        .map_err(|e| CatalogError::SignatureFile(e.to_string()))?;
    let key = trusted
        .iter()
        .find(|k| k.to_string() == signature.key_id)
        .ok_or(CatalogError::UntrustedKey)?;
    let bytes = unhex::<64>(&signature.signature)
        .ok_or_else(|| CatalogError::SignatureFile("malformed signature".into()))?;
    let message = [CONTEXT, index].concat();
    key.0
        .verify_strict(&message, &ed25519_dalek::Signature::from_bytes(&bytes))
        .map_err(|_| CatalogError::BadSignature)?;
    Catalog::parse(index)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Converter {
    /// `meshway-rules` crate version that built the artifacts.
    pub version: String,
    pub format_major: u16,
    pub format_minor: u16,
}

impl Converter {
    pub fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            format_major: MAJOR,
            format_minor: MINOR,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Upstream {
    pub repo: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub commit: String,
}

/// Where a set's source came from; provenance stays out of the artifact.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Location {
    pub repo: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub commit: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFacts {
    pub format: Format,
    /// SHA-256 of the source bytes, lowercase hex.
    pub sha256: String,
    pub bytes: u64,
}

/// What one conversion produced (`meshway-rules convert` prints it).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversionRecord {
    pub digest: Digest,
    pub bytes: u64,
    pub entries: u64,
    /// `MWRS` major version, as in the netmap reference.
    pub format: u16,
    pub counts: Counts,
    pub source: SourceFacts,
    pub report: Report,
}

impl ConversionRecord {
    pub fn new(source: &[u8], format: Format, conversion: &Conversion) -> Self {
        let artifact = &conversion.artifact;
        Self {
            digest: artifact.digest(),
            bytes: artifact.bytes().len() as u64,
            entries: artifact.counts().entries(),
            format: MAJOR,
            counts: artifact.counts(),
            source: SourceFacts {
                format,
                sha256: Digest::of(source).hex(),
                bytes: source.len() as u64,
            },
            report: conversion.report.clone(),
        }
    }
}

/// `sets/<collection>/<name>.json`: a conversion plus its upstream location,
/// written by the pipeline.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetRecord {
    pub upstream: Location,
    pub conversion: ConversionRecord,
    /// The pipeline kept the previous artifact because this set's latest
    /// conversion failed or looked suspicious; absent for a current set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

/// Why a catalog still publishes an older artifact for one set.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Held {
    pub since: DateTime<Utc>,
    pub reason: String,
}

/// One `index.json` set; the report is the summary (items stay in the record).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CatalogSet {
    pub digest: Digest,
    pub bytes: u64,
    pub entries: u64,
    pub format: u16,
    pub counts: Counts,
    pub upstream: Location,
    pub source: SourceFacts,
    pub report: Summary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
}

impl CatalogSet {
    /// The artifact a catalog entry names must be exactly that artifact.
    pub fn check(&self, set: &RuleSet) -> Result<(), String> {
        if set.digest() != self.digest {
            return Err(format!(
                "artifact digest is {}, expected {}",
                set.digest(),
                self.digest
            ));
        }
        if set.as_bytes().len() as u64 != self.bytes
            || set.counts() != self.counts
            || set.counts().entries() != self.entries
            || self.format != MAJOR
        {
            return Err("artifact facts differ from the catalog entry".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    /// Strictly increasing per catalog; consumers reject a lower value.
    pub sequence: u64,
    pub generated_at: DateTime<Utc>,
    pub converter: Converter,
    pub upstreams: Vec<Upstream>,
    /// Keyed `<collection>/<name>`.
    pub sets: BTreeMap<String, CatalogSet>,
}

/// `<collection>/<name>`: collection `[a-z0-9][a-z0-9-]{0,31}`, name
/// `[a-z0-9][a-z0-9!@._+-]{0,127}`.
pub fn valid_set_name(key: &str) -> bool {
    let Some((collection, name)) = key.split_once('/') else {
        return false;
    };
    let starts = |s: &str| {
        s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    };
    starts(collection)
        && collection.len() <= 32
        && collection
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && starts(name)
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"!@._+-".contains(&b))
}

impl Catalog {
    pub fn parse(index: &[u8]) -> Result<Self, CatalogError> {
        let catalog: Self =
            serde_json::from_slice(index).map_err(|e| CatalogError::Document(e.to_string()))?;
        if catalog.schema != SCHEMA {
            return Err(CatalogError::Schema(catalog.schema));
        }
        if let Some(bad) = catalog.sets.keys().find(|k| !valid_set_name(k)) {
            return Err(CatalogError::SetName(bad.clone()));
        }
        Ok(catalog)
    }

    /// Exact bytes to publish as `index.json`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).expect("catalog serializes");
        bytes.push(b'\n');
        bytes
    }

    /// Builds the next catalog from per-set records. Every artifact is opened
    /// through `artifact` and must match its record; the sequence is one past
    /// `previous`, or 1 for a new catalog.
    pub fn assemble<E: fmt::Display>(
        previous: Option<&Catalog>,
        records: BTreeMap<String, SetRecord>,
        generated_at: DateTime<Utc>,
        mut artifact: impl FnMut(&Digest) -> Result<RuleSet, E>,
    ) -> Result<Self, CatalogError> {
        if records.is_empty() {
            return Err(CatalogError::Empty);
        }
        let mut upstreams: BTreeMap<(String, String), String> = BTreeMap::new();
        let mut sets = BTreeMap::new();
        for (name, record) in records {
            if !valid_set_name(&name) {
                return Err(CatalogError::SetName(name));
            }
            let SetRecord {
                upstream,
                conversion: c,
                held,
            } = record;
            let entry = CatalogSet {
                digest: c.digest,
                bytes: c.bytes,
                entries: c.entries,
                format: c.format,
                counts: c.counts,
                upstream,
                source: c.source,
                report: c.report.summary,
                held,
            };
            let problem = |problem: String| CatalogError::Set {
                name: name.clone(),
                problem,
            };
            let set = artifact(&entry.digest).map_err(|e| problem(e.to_string()))?;
            entry.check(&set).map_err(problem)?;
            let key = (
                entry.upstream.repo.clone(),
                entry.upstream.reference.clone(),
            );
            match upstreams.get(&key) {
                Some(commit) if *commit != entry.upstream.commit => {
                    return Err(CatalogError::Upstream {
                        repo: key.0,
                        reference: key.1,
                    });
                }
                _ => {
                    upstreams.insert(key, entry.upstream.commit.clone());
                }
            }
            sets.insert(name, entry);
        }
        Ok(Self {
            schema: SCHEMA,
            sequence: previous.map_or(1, |p| p.sequence + 1),
            generated_at,
            converter: Converter::current(),
            upstreams: upstreams
                .into_iter()
                .map(|((repo, reference), commit)| Upstream {
                    repo,
                    reference,
                    commit,
                })
                .collect(),
            sets,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::convert;

    fn record(source: &[u8]) -> (SetRecord, Vec<u8>) {
        let conversion = convert(source, Format::MihomoDomainText).unwrap();
        let bytes = conversion.artifact.bytes().to_vec();
        (
            SetRecord {
                upstream: Location {
                    repo: "MetaCubeX/meta-rules-dat".into(),
                    reference: "meta".into(),
                    commit: "83c8208bdfae92b1caaa29745f51f233b6f1847e".into(),
                    path: "geo/geosite/openai.list".into(),
                },
                conversion: ConversionRecord::new(source, Format::MihomoDomainText, &conversion),
                held: None,
            },
            bytes,
        )
    }

    #[test]
    fn sign_assemble_verify() {
        let key = signing_key(&"11".repeat(32)).unwrap();
        let other = signing_key(&"22".repeat(32)).unwrap();
        let (rec, bytes) = record(b"+.openai.com\nchatgpt.com\n");
        let open = |_: &Digest| RuleSet::from_vec(bytes.clone());
        let records: BTreeMap<_, _> = [("meta-geosite/openai".to_owned(), rec.clone())].into();
        let first = Catalog::assemble(None, records.clone(), Utc::now(), open).unwrap();
        assert_eq!(first.sequence, 1);
        let second = Catalog::assemble(Some(&first), records, Utc::now(), open).unwrap();
        assert_eq!(second.sequence, 2);
        let index = second.to_bytes();
        let sig = serde_json::to_vec(&sign(&index, &key)).unwrap();
        let trusted = [public_key(&other), public_key(&key)];
        assert_eq!(verify(&index, &sig, &trusted).unwrap(), second);
        assert!(matches!(
            verify(&index, &sig, &trusted[..1]),
            Err(CatalogError::UntrustedKey)
        ));
        let mut tampered = index.clone();
        tampered[10] ^= 1;
        assert!(matches!(
            verify(&tampered, &sig, &trusted),
            Err(CatalogError::BadSignature)
        ));
        let mismatched = |_: &Digest| RuleSet::from_vec(record(b"other.example\n").1);
        let records: BTreeMap<_, _> = [("meta-geosite/openai".to_owned(), rec)].into();
        assert!(matches!(
            Catalog::assemble(None, records, Utc::now(), mismatched),
            Err(CatalogError::Set { .. })
        ));
    }

    #[test]
    fn keys_and_names() {
        let key = public_key(&signing_key(&"11".repeat(32)).unwrap());
        let text = key.to_string();
        assert_eq!(text.parse::<PublicKey>(), Ok(key));
        assert_eq!(text["ed25519:".len()..].parse::<PublicKey>(), Ok(key));
        assert!(text.to_uppercase().parse::<PublicKey>().is_err());
        for good in [
            "meta-geosite/geolocation-!cn",
            "meta-geoip/cn",
            "meta-geosite/google@cn",
        ] {
            assert!(valid_set_name(good), "{good}");
        }
        for bad in [
            "cn", "meta/", "/cn", "Meta/cn", "meta/c n", "a/b/c", "meta/-x",
        ] {
            assert!(!valid_set_name(bad), "{bad}");
        }
    }
}
