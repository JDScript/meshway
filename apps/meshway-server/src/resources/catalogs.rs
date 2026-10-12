//! Signed rule-set catalogs (`index.json` and `index.json.sig`) and their
//! artifacts. Verification uses the provider's trusted keys; the accepted
//! `sequence` is the Server's persistent state and is checked when the
//! content is bound (`store::resources::artifacts::accept_catalog`).
use super::{
    download::{Connector, download},
    rulesets::{CatalogAcceptance, Prepared, Record, trust},
};
use meshway_configuration::{RuleSetProvider, providers::catalog_set_name};
use meshway_protocol::resources::ResourceProblem;
use meshway_rules::{
    MAX_ARTIFACT_BYTES, Report, RuleSet,
    catalog::{self, Catalog, CatalogError, PublicKey, SetRecord},
    convert::Format,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
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
type Verified = BTreeMap<(String, String), Arc<Catalog>>;

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
    ) -> Result<Arc<Catalog>, ResourceProblem> {
        let key = (provider.catalog.clone(), trust(&provider.public_keys));
        if let Some(catalog) = self
            .verified
            .lock()
            .map_err(|_| ResourceProblem::BudgetExceeded)?
            .get(&key)
        {
            return Ok(Arc::clone(catalog));
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
        let catalog = Arc::new(catalog::verify(&index, &signature, &keys).map_err(verify_error)?);
        self.verified
            .lock()
            .map_err(|_| ResourceProblem::BudgetExceeded)?
            .insert(key, Arc::clone(&catalog));
        Ok(catalog)
    }

    /// The artifact of `<collection>/<name>` from a verified catalog, checked
    /// against its entry, with the set's published report when readable.
    pub(crate) async fn fetch(
        &self,
        provider: &RuleSetProvider,
        name: &str,
        connector: &dyn Connector,
    ) -> Result<Prepared, ResourceProblem> {
        let catalog = self.catalog(provider, connector).await?;
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
        let artifact = tokio::task::spawn_blocking(move || {
            let set = RuleSet::from_vec(bytes).map_err(|_| ResourceProblem::IntegrityFailed)?;
            checked
                .check(&set)
                .map_err(|_| ResourceProblem::IntegrityFailed)?;
            // Rebuilding proves the artifact is the canonical one for its members.
            meshway_rules::convert::convert(set.as_bytes(), Format::Meshway)
                .map(|conversion| conversion.artifact)
                .map_err(|_| ResourceProblem::IntegrityFailed)
        })
        .await
        .map_err(|_| ResourceProblem::BudgetExceeded)??;
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
            }),
        })
    }
}
