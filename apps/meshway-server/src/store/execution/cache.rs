//! Private versioned publication envelopes. Obsolete bodies are evidence, never
//! execution configurations; no old runtime or authorization is reconstructed.
use super::{id, now};
use crate::{error::Error, secrets::Sealer};
use meshway_configuration::execution::{CompiledNode, CompiledPublication};
use meshway_model::{ConfigurationRef, Digest, NetworkId, NodeId, PublicationId, Revision};
use rusqlite::{Transaction, params};
use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

const LIMIT: usize = 16 * 1024 * 1024;

pub(super) enum Stored {
    Current(CompiledPublication),
    Obsolete {
        revision: Revision,
        nodes: BTreeMap<NodeId, ConfigurationRef>,
    },
}

/// A reference can be admitted as historical evidence without inventing an
/// installation context for a configuration this compiler cannot understand.
pub(crate) struct HistoricalTarget {
    pub reference: ConfigurationRef,
    pub current: Option<CompiledNode>,
}

impl Stored {
    pub(super) fn current(self) -> Option<CompiledPublication> {
        match self {
            Self::Current(value) => Some(value),
            Self::Obsolete { .. } => None,
        }
    }

    pub(super) fn historical(self, node: NodeId) -> Option<HistoricalTarget> {
        match self {
            Self::Current(mut value) => value.nodes.remove(&node).map(|current| HistoricalTarget {
                reference: ConfigurationRef {
                    revision: current.configuration.revision,
                    execution_publication: current.configuration.publication,
                    digest: current.digest,
                },
                current: Some(current),
            }),
            Self::Obsolete { mut nodes, .. } => {
                nodes.remove(&node).map(|reference| HistoricalTarget {
                    reference,
                    current: None,
                })
            }
        }
    }

    pub(super) fn references(self) -> (Revision, BTreeMap<NodeId, ConfigurationRef>) {
        match self {
            Self::Current(value) => (
                value.revision,
                value
                    .nodes
                    .into_iter()
                    .map(|(node, slice)| {
                        (
                            node,
                            ConfigurationRef {
                                revision: value.revision,
                                execution_publication: value.publication,
                                digest: slice.digest,
                            },
                        )
                    })
                    .collect(),
            ),
            Self::Obsolete { revision, nodes } => (revision, nodes),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Envelope<'a> {
    network_id: NetworkId,
    revision: Revision,
    publication: PublicationId,
    dependency_digest: Digest,
    compiler_version: String,
    #[serde(borrow, deserialize_with = "meshway_serde::unique_map")]
    nodes: BTreeMap<NodeId, Slice<'a>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Slice<'a> {
    digest: Digest,
    #[serde(borrow)]
    configuration: &'a RawValue,
}

fn decode<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = T::deserialize(meshway_serde::MapOnly(&mut decoder))
        .map_err(|_| Error::decode("execution cache"))?;
    decoder
        .end()
        .map_err(|_| Error::decode("execution cache"))?;
    Ok(value)
}

/// Every publication consumer passes through this seal/context/header/digest
/// check. The compiler marker is the only version discriminator, never a failed
/// current deserialization. Raw values borrow the zeroizing decrypted buffer.
pub(super) fn read(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    publication: PublicationId,
    payload: &[u8],
) -> Result<Stored, Error> {
    if payload.len() > LIMIT + 128 {
        return Err(Error::decode("execution cache limit"));
    }
    let plain = sealer.open_execution(network, publication, payload)?;
    if plain.as_bytes().len() > LIMIT {
        return Err(Error::decode("execution cache limit"));
    }
    let envelope: Envelope<'_> = decode(plain.as_bytes())?;
    if envelope.nodes.len() > 10_000 {
        return Err(Error::decode("execution cache node limit"));
    }
    let (revision, dependency): (i64, Vec<u8>) = tx.query_row(
        "SELECT revision,dependency_digest FROM execution_publications WHERE network_id=?1 AND id=?2",
        params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if envelope.network_id != network
        || envelope.publication != publication
        || envelope.revision.get() as i64 != revision
        || envelope.dependency_digest.as_bytes() != dependency.as_slice()
    {
        return Err(Error::decode("execution cache header"));
    }
    if envelope.compiler_version == meshway_policy::compile::COMPILER_VERSION {
        let current: CompiledPublication = decode(plain.as_bytes())?;
        for (node, slice) in &current.nodes {
            let cfg = &slice.configuration;
            if cfg.network_id != network
                || cfg.node_id != *node
                || cfg.publication != publication
                || cfg.revision != current.revision
                || cfg.dependency_digest != current.dependency_digest
                || meshway_configuration::digest(cfg)? != slice.digest
            {
                return Err(Error::decode("execution cache slice"));
            }
        }
        return Ok(Stored::Current(current));
    }
    if !matches!(
        envelope.compiler_version.as_str(),
        "meshway-demand-1" | "meshway-demand-2"
    ) {
        return Err(Error::decode("execution cache version"));
    }
    let mut references = BTreeMap::new();
    for (node, slice) in &envelope.nodes {
        validate_obsolete(&envelope, *node, slice)?;
        references.insert(
            *node,
            ConfigurationRef {
                revision: envelope.revision,
                execution_publication: publication,
                digest: slice.digest,
            },
        );
    }
    Ok(Stored::Obsolete {
        revision: envelope.revision,
        nodes: references,
    })
}

fn validate_obsolete(
    envelope: &Envelope<'_>,
    node: NodeId,
    slice: &Slice<'_>,
) -> Result<(), Error> {
    // Traverse opaque fields too: serde's ordinary IgnoredAny alone would hide
    // duplicates. MapOnly carries duplicate checks through the entire tree.
    let _: serde::de::IgnoredAny = decode(slice.configuration.get().as_bytes())?;
    let fields: BTreeMap<String, &RawValue> = decode(slice.configuration.get().as_bytes())?;
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
        return Err(Error::decode("obsolete execution cache fields"));
    }
    validate_old_fields(&fields)?;
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Header {
        network_id: NetworkId,
        node_id: NodeId,
        revision: Revision,
        publication: PublicationId,
        dependency_digest: Digest,
        self_keys: meshway_configuration::execution::TrustedNode,
    }
    let header: Header = decode(slice.configuration.get().as_bytes())?;
    if header.network_id != envelope.network_id
        || header.node_id != node
        || header.revision != envelope.revision
        || header.publication != envelope.publication
        || header.dependency_digest != envelope.dependency_digest
        || header.self_keys.control_key.public_key.algorithm != meshway_model::KeyAlgorithm::Ed25519
        || header.self_keys.dataplane_key.public_key.algorithm
            != meshway_model::KeyAlgorithm::X25519
        || slice.digest.as_bytes()
            != Sha256::digest(slice.configuration.get().as_bytes()).as_slice()
    {
        return Err(Error::decode("obsolete execution cache slice"));
    }
    Ok(())
}

fn validate_old_fields(fields: &BTreeMap<String, &RawValue>) -> Result<(), Error> {
    use meshway_configuration::execution::*;
    use meshway_configuration::{
        DnsConfiguration, NodeAddresses, NodeIdentity, transport::PreparedListener,
    };
    use meshway_model::{ObservationId, RequestId};
    fn field<T: serde::de::DeserializeOwned>(
        fields: &BTreeMap<String, &RawValue>,
        key: &str,
    ) -> Result<(), Error> {
        serde_json::from_str::<T>(
            fields
                .get(key)
                .ok_or(Error::decode("obsolete execution cache field"))?
                .get(),
        )
        .map(|_| ())
        .map_err(|_| Error::decode("obsolete execution cache field"))
    }
    fn map<K: serde::de::DeserializeOwned + Ord, V: serde::de::DeserializeOwned>(
        fields: &BTreeMap<String, &RawValue>,
        key: &str,
    ) -> Result<(), Error> {
        let mut decoder = serde_json::Deserializer::from_str(
            fields
                .get(key)
                .ok_or(Error::decode("obsolete execution cache field"))?
                .get(),
        );
        let _: BTreeMap<K, V> = meshway_serde::unique_map(&mut decoder)
            .map_err(|_| Error::decode("obsolete execution cache map"))?;
        decoder
            .end()
            .map_err(|_| Error::decode("obsolete execution cache map"))
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
            .ok_or(Error::decode("obsolete DNS cache"))?
            .get()
            .as_bytes(),
    )?;
    if dns.len() != 4 {
        return Err(Error::decode("obsolete DNS cache"));
    }
    field::<DnsConfiguration>(&dns, "settings")?;
    map::<String, NodeAddresses>(&dns, "meshHosts")?;
    field::<Vec<TrustedDnsAssociation>>(&dns, "associations")?;
    field::<String>(&dns, "profile")
}

pub(super) fn retire(
    tx: &Transaction<'_>,
    network: NetworkId,
    publication: PublicationId,
    now: i64,
) -> Result<(), Error> {
    tx.execute(
        "UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL WHERE network_id=?1 AND target_publication_id=?2",
        params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice()],
    )?;
    tx.execute(
        "UPDATE execution_publications SET withdrawn_at=COALESCE(withdrawn_at,?3) WHERE network_id=?1 AND id=?2",
        params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice(), now],
    )?;
    Ok(())
}

/// Runs in the SQLite startup owner's transaction, before services are exposed.
/// Any unknown/corrupt row rolls back every retirement; sealed history is never
/// rewritten, and source revisions, identities and demands are not changed.
pub(crate) fn retire_obsolete(tx: &Transaction<'_>, sealer: &Sealer) -> Result<(), Error> {
    let now = now()?;
    let mut statement = tx.prepare(
        "SELECT network_id,id,payload FROM execution_publications ORDER BY network_id,sequence",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let network = id(row.get(0)?)?;
        let publication = id(row.get(1)?)?;
        let payload: Vec<u8> = row.get(2)?;
        if matches!(
            read(tx, sealer, network, publication, &payload)?,
            Stored::Obsolete { .. }
        ) {
            retire(tx, network, publication, now)?;
        }
    }
    Ok(())
}
