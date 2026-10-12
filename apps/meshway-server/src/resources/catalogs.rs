//! Signed rule-set catalogs (`index.json` and `index.json.sig`) and their
//! artifacts. Verification uses the provider's trusted keys; the accepted
//! `sequence` is the Server's persistent state and is checked when the
//! content is bound (`store::resources::artifacts::accept_catalog`).
use super::{
    download::{Connector, download},
    rulesets::{CatalogAcceptance, Prepared, Record, trust},
};
use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_configuration::{RuleSetProvider, providers::catalog_set_name};
use meshway_protocol::resources::ResourceProblem;
use meshway_rules::{
    MAX_ARTIFACT_BYTES, Report, RuleSet,
    catalog::{self, Catalog, CatalogError, PublicKey, SetRecord},
    convert::Format,
};
use rusqlite::Transaction;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock, Mutex},
};

/// Largest accepted `index.json`.
pub(crate) const MAX_CATALOG_BYTES: usize = meshway_rules::convert::MAX_SOURCE_BYTES;
const MAX_SIGNATURE_BYTES: usize = 4096;
const MAX_RECORD_BYTES: usize = 4 << 20;

/// URLs beside a catalog document: its signature and an artifact.
fn sibling(catalog: &str, name: &str) -> Result<String, ResourceProblem> {
    let base = catalog
        .rsplit_once('/')
        .map(|(base, _)| base)
        .ok_or(ResourceProblem::InvalidSource)?;
    Ok(format!("{base}/{name}"))
}

/// Catalogs verified for one preparation: the sets of one edit share one
/// download of each catalog; every refresh starts with an empty memo.
/// Verified catalogs by (catalog URL, trusted key set).
type Verified = BTreeMap<(String, String), VerifiedCatalog>;

/// One verified catalog with the exact bytes that verified.
#[derive(Clone)]
struct VerifiedCatalog {
    catalog: Arc<Catalog>,
    document: Arc<[u8]>,
    signature: Arc<[u8]>,
}

#[derive(Clone, Default)]
pub(crate) struct Catalogs {
    verified: Arc<Mutex<Verified>>,
}

fn verify_error(error: CatalogError) -> ResourceProblem {
    match error {
        CatalogError::UntrustedKey
        | CatalogError::BadSignature
        | CatalogError::SignatureFile(_) => ResourceProblem::SignatureInvalid,
        _ => ResourceProblem::InvalidSource,
    }
}

impl Catalogs {
    async fn catalog(
        &self,
        provider: &RuleSetProvider,
        connector: &dyn Connector,
    ) -> Result<VerifiedCatalog, ResourceProblem> {
        let key = (provider.catalog.clone(), trust(&provider.public_keys));
        if let Some(verified) = self
            .verified
            .lock()
            .map_err(|_| ResourceProblem::BudgetExceeded)?
            .get(&key)
        {
            return Ok(verified.clone());
        }
        let keys = provider
            .public_keys
            .iter()
            .map(|key| key.parse::<PublicKey>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ResourceProblem::SignatureInvalid)?;
        let index = download(&provider.catalog, connector, MAX_CATALOG_BYTES).await?;
        let signature = download(
            &sibling(&provider.catalog, catalog::SIGNATURE)?,
            connector,
            MAX_SIGNATURE_BYTES,
        )
        .await?;
        let verified = VerifiedCatalog {
            catalog: Arc::new(catalog::verify(&index, &signature, &keys).map_err(verify_error)?),
            document: index.into(),
            signature: signature.into(),
        };
        self.verified
            .lock()
            .map_err(|_| ResourceProblem::BudgetExceeded)?
            .insert(key, verified.clone());
        Ok(verified)
    }

    /// The artifact of `<collection>/<name>` from a verified catalog, checked
    /// against its entry, with the set's published report when readable.
    pub(crate) async fn fetch(
        &self,
        provider: &RuleSetProvider,
        name: &str,
        connector: &dyn Connector,
        workers: &super::downloads::Workers,
    ) -> Result<Prepared, ResourceProblem> {
        let verified = self.catalog(provider, connector).await?;
        let catalog = &verified.catalog;
        let set_name = catalog_set_name(provider, name);
        let entry = catalog
            .sets
            .get(&set_name)
            .ok_or(ResourceProblem::UnknownRuleSet)?;
        let bytes = download(
            &sibling(
                &provider.catalog,
                &format!("{}/{}", catalog::ARTIFACTS, entry.digest.file_name()),
            )?,
            connector,
            MAX_ARTIFACT_BYTES,
        )
        .await?;
        let checked = entry.clone();
        let artifact = workers
            .run(move || {
                let set = RuleSet::from_vec(bytes).map_err(|_| ResourceProblem::IntegrityFailed)?;
                checked
                    .check(&set)
                    .map_err(|_| ResourceProblem::IntegrityFailed)?;
                // Rebuilding proves the artifact is the canonical one for its members.
                meshway_rules::convert::convert(set.as_bytes(), Format::Meshway)
                    .map(|conversion| conversion.artifact)
                    .map_err(|_| ResourceProblem::IntegrityFailed)
            })
            .await?;
        let published = match download(
            &sibling(
                &provider.catalog,
                &format!("{}/{set_name}.json", catalog::SETS),
            )?,
            connector,
            MAX_RECORD_BYTES,
        )
        .await
        {
            Ok(bytes) => serde_json::from_slice::<SetRecord>(&bytes)
                .ok()
                .filter(|record| {
                    record.conversion.digest == entry.digest
                        && record.conversion.report.summary == entry.report
                })
                .map(|record| record.conversion.report),
            Err(_) => None,
        };
        Ok(Prepared {
            artifact,
            record: Record {
                format: entry.source.format.to_string(),
                upstream: Some(entry.upstream.clone()),
                sequence: Some(catalog.sequence),
                held: entry.held.clone(),
                report: published.unwrap_or_else(|| Report {
                    summary: entry.report.clone(),
                    items: Vec::new(),
                }),
            },
            catalog: Some(CatalogAcceptance {
                catalog: provider.catalog.clone(),
                trust: trust(&provider.public_keys),
                sequence: catalog.sequence,
                document: Arc::clone(&verified.document),
                signature: Arc::clone(&verified.signature),
            }),
        })
    }
}

/// A catalog this Server accepted, as stored: the newest verified sequence.
pub(crate) struct Accepted {
    pub catalog: Arc<Catalog>,
    pub verified_at: DateTime<Utc>,
}

/// Parsed accepted catalogs by (catalog URL, trust), with the stored row's
/// (sequence, verification time): every acceptance rewrites both, so an
/// entry is replaced whenever the stored document may have changed.
type Parsed = BTreeMap<(String, String), ((u64, i64), Arc<Catalog>)>;
static ACCEPTED: LazyLock<Mutex<Parsed>> = LazyLock::new(Mutex::default);
const MAX_PARSED: usize = 16;

/// The provider's accepted catalog, verified again against the provider's
/// keys when first parsed in this process; `None` before this Server ever
/// verified it. Never downloads.
pub(crate) fn accepted(
    tx: &Transaction<'_>,
    provider: &RuleSetProvider,
) -> Result<Option<Accepted>, Error> {
    let key = (provider.catalog.clone(), trust(&provider.public_keys));
    let Some((sequence, at)) =
        crate::store::resources::artifacts::catalog_head(tx, &key.0, &key.1)?
    else {
        return Ok(None);
    };
    let verified_at = DateTime::from_timestamp_millis(at)
        .ok_or(Error::decode("rule_set_catalogs.verified_at"))?;
    if let Some((parsed, catalog)) = ACCEPTED.lock().map_err(|_| Error::RUNTIME)?.get(&key)
        && *parsed == (sequence, at)
    {
        return Ok(Some(Accepted {
            catalog: Arc::clone(catalog),
            verified_at,
        }));
    }
    let (document, signature) =
        crate::store::resources::artifacts::catalog_document(tx, &key.0, &key.1)?;
    let keys = provider
        .public_keys
        .iter()
        .map(|key| key.parse::<PublicKey>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::decode("rule_set_catalogs.trust"))?;
    let catalog = catalog::verify(&document, &signature, &keys)
        .ok()
        .filter(|catalog| catalog.sequence == sequence)
        .map(Arc::new)
        .ok_or(Error::decode("rule_set_catalogs.document"))?;
    let mut parsed = ACCEPTED.lock().map_err(|_| Error::RUNTIME)?;
    if parsed.len() >= MAX_PARSED && !parsed.contains_key(&key) {
        parsed.clear();
    }
    parsed.insert(key, ((sequence, at), Arc::clone(&catalog)));
    Ok(Some(Accepted {
        catalog,
        verified_at,
    }))
}
