//! Authenticated network use cases and their transaction inputs.
//!
//! Membership is checked with the caller's current session for every operation.
//! Configuration remains a versioned value, never a second set of entity fields.

use crate::effect::Effect;
use crate::error::Error;
use chrono::{DateTime, Utc};
pub use meshway_model::{MemberSummary, NetworkSummary};
use meshway_model::{NetworkId, RequestId};
use rusqlite::TransactionBehavior;
use std::sync::Arc;

pub(crate) mod configuration;
pub(crate) mod management;
pub(crate) mod pagination;

use crate::{
    accounts::token_digest,
    secrets::Sealer,
    store::{Handle, networks as store},
};

#[derive(Clone)]
pub struct Networks {
    handle: Handle,
    sealer: Arc<Sealer>,
    committer: crate::configuration::Committer,
}
pub(crate) struct NameChange {
    pub(crate) name: String,
    pub(crate) expected: String,
    pub(crate) request: RequestId,
}

impl Networks {
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        committer: crate::configuration::Committer,
    ) -> Self {
        Self {
            handle,
            sealer,
            committer,
        }
    }

    pub async fn create(
        &self,
        token: &str,
        name: String,
        request_id: RequestId,
    ) -> Result<NetworkSummary, Error> {
        let digest = token_digest(token)?;
        let name = normalize_name(&name)?;
        // Cheap authentication before template preparation; the creating
        // transaction still checks the current session again after admission.
        self.handle
            .read(move |tx| store::authenticate(tx, digest))
            .await?;
        let prepared = configuration::prepare(name, &self.sealer)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.committer.cache);
        self.handle
            .write(move |tx| store::create(tx, digest, prepared, request_id, &sealer, &cache))
            .await
    }

    pub async fn list(
        &self,
        token: &str,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<NetworkPage, Error> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.committer.cache);
        self.handle
            .read(move |tx| store::list(tx, digest, limit, cursor, &sealer, &cache))
            .await
    }

    pub async fn get(&self, token: &str, network_id: NetworkId) -> Result<NetworkSummary, Error> {
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.committer.cache);
        self.handle
            .read(move |tx| store::get(tx, digest, network_id, &sealer, &cache))
            .await
    }

    pub async fn rename(
        &self,
        token: &str,
        network_id: NetworkId,
        name: String,
        expected_name: String,
        request_id: RequestId,
    ) -> Result<NetworkSummary, Error> {
        let digest = token_digest(token)?;
        let name = normalize_name(&name)?;
        // Expected metadata must be the exact previously returned spelling.
        if normalize_name(&expected_name)? != expected_name {
            return Err(Error::InvalidName);
        }
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.committer.cache);
        let changed = name != expected_name;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let result = store::rename(
                    tx,
                    digest,
                    network_id,
                    NameChange {
                        name,
                        expected: expected_name,
                        request: request_id,
                    },
                    &sealer,
                    &cache,
                )?;
                let effects = if changed {
                    vec![Effect::Authorization { network_id }]
                } else {
                    Vec::new()
                };
                Ok((result, effects))
            })
            .await
    }

    pub async fn members(
        &self,
        token: &str,
        network_id: NetworkId,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<MemberPage, Error> {
        pagination::validate_limit(limit)?;
        let digest = token_digest(token)?;
        let sealer = Arc::clone(&self.sealer);
        let cache = Arc::clone(&self.committer.cache);
        self.handle
            .read(move |tx| store::members(tx, digest, network_id, limit, cursor, &sealer, &cache))
            .await
    }
}

/// Creation-bounded traversal, with current metadata and permissions on each
/// page. `as_of` identifies the first page's boundary, not a frozen SQL snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPage {
    pub items: Vec<NetworkSummary>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
    pub total: meshway_model::Count,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberPage {
    pub items: Vec<MemberSummary>,
    pub next_cursor: Option<String>,
    pub as_of: DateTime<Utc>,
    pub total: meshway_model::Count,
}

pub(crate) fn normalize_name(value: &str) -> Result<String, Error> {
    let rules = meshway_protocol::NetworkNameRules::CURRENT;
    if value.len() > usize::from(rules.max_input_utf8_bytes)
        || (rules.forbid_control_characters && value.chars().any(char::is_control))
    {
        return Err(Error::InvalidName);
    }
    let value = value.trim();
    if !(usize::from(rules.min_codepoints)..=usize::from(rules.max_codepoints))
        .contains(&value.chars().count())
    {
        return Err(Error::InvalidName);
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests;
