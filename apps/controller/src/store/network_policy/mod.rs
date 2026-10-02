//! The sole network policy source, JSONC draft fidelity and revision CAS.
use super::{NetworkId, Store, configuration, publication};
use crate::{
    Error,
    contracts::network_policy::{NetworkPolicySnapshot, SaveNetworkPolicyRequest},
};
use meshway_policy::{AutomaticRouting, NetworkPolicy};
use meshway_protocol::MAX_SOURCE_JSONC_BYTES;
use rusqlite::{Connection, TransactionBehavior, params};

pub(super) fn initialize(db: &Connection, network: &NetworkId) -> Result<(), Error> {
    let json = serde_json::to_string(&NetworkPolicy::default_template())
        .map_err(|_| Error::Configuration("cannot encode default policy"))?;
    db.execute(
        "INSERT INTO network_policy(network_id,policy,source_jsonc) VALUES(?1,?2,NULL)",
        params![network.as_str(), json],
    )?;
    Ok(())
}

pub(super) fn source(
    db: &Connection,
    network: &NetworkId,
) -> Result<(NetworkPolicy, Option<String>), Error> {
    let (json, text): (String, Option<String>) = db.query_row(
        "SELECT policy,source_jsonc FROM network_policy WHERE network_id=?1",
        [network.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((
        serde_json::from_str(&json)
            .map_err(|_| Error::Configuration("invalid persisted policy"))?,
        text,
    ))
}

fn snapshot(db: &Connection, network: &NetworkId) -> Result<NetworkPolicySnapshot, Error> {
    let (policy, source_jsonc) = source(db, network)?;
    let compiled = publication::mesh_execution::Publication::read(db, network, &policy)?;
    Ok(NetworkPolicySnapshot {
        network_id: network.as_str().to_owned(),
        revision: configuration::revision(db, network)?,
        policy,
        source_jsonc,
        valid: compiled.valid(),
        diagnostics: compiled.diagnostics(),
    })
}

impl Store {
    pub fn network_policy(&self, network: &NetworkId) -> Result<NetworkPolicySnapshot, Error> {
        snapshot(&self.connection, network)
    }

    pub fn save_network_policy(
        &mut self,
        network: &NetworkId,
        request: &SaveNetworkPolicyRequest,
    ) -> Result<NetworkPolicySnapshot, Error> {
        if serde_json::to_vec(&request.policy)
            .map_err(|_| Error::Invalid("cannot encode policy"))?
            .len()
            > MAX_SOURCE_JSONC_BYTES
            || request
                .source_jsonc
                .as_ref()
                .is_some_and(|s| s.len() > MAX_SOURCE_JSONC_BYTES)
        {
            return Err(Error::Unprocessable("policy source exceeds 256 KiB"));
        }
        if let Some(text) = &request.source_jsonc {
            let parsed: NetworkPolicy = super::jsonc::parse_source(text)?;
            if parsed != request.policy {
                return Err(Error::Unprocessable("source_jsonc differs from policy"));
            }
        }
        // Current execution supports IPv4 Mesh only. Unsupported external intent
        // is rejected instead of silently dropping its semantics.
        if !request.policy.public_rules.is_empty()
            || !request.policy.profiles.is_empty()
            || !request.policy.assignments.is_empty()
            || request.policy.automatic != AutomaticRouting::default()
            || request.policy.grants.iter().flat_map(|g| &g.dst).any(|s| {
                s == "autogroup:internet" || s.starts_with("ip:") || s.starts_with("domain:")
            })
        {
            return Err(Error::Unprocessable(
                "Mesh execution supports connections, grants and Mesh routes only; external profiles are not implemented",
            ));
        }
        let tx = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&tx, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let compiled =
            publication::mesh_execution::Publication::read(&tx, network, &request.policy)?;
        if !compiled.valid() {
            return Err(Error::Unprocessable("invalid policy references or source"));
        }
        if source(&tx, network)? != (request.policy.clone(), request.source_jsonc.clone()) {
            let json = serde_json::to_string(&request.policy)
                .map_err(|_| Error::Configuration("cannot encode policy"))?;
            tx.execute(
                "UPDATE network_policy SET policy=?2,source_jsonc=?3 WHERE network_id=?1",
                params![network.as_str(), json, request.source_jsonc],
            )?;
            configuration::bump(&tx, network)?;
            publication::finish_change(&tx, network, publication::ChangeKind::Adjacency)?;
        }
        let result = snapshot(&tx, network)?;
        tx.commit()?;
        Ok(result)
    }
}

#[cfg(test)]
mod publication_regressions;
#[cfg(test)]
mod tests;
