//! Source storage and epoch negotiation; execution projection is publication-owned.
use super::{NetworkId, Store, configuration};
use crate::{
    Error,
    policy_v3_api::{NetworkPolicySnapshot, SaveNetworkPolicyRequest},
};
use meshway_policy::v3::{self, NetworkPolicy};
use meshway_protocol::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(super) fn source(
    db: &Connection,
    network: &NetworkId,
) -> Result<Option<(NetworkPolicy, Option<String>)>, Error> {
    let row: Option<(String, Option<String>)> = db.query_row("SELECT policy,source_jsonc FROM network_policy_sources WHERE network_id=?1 AND source_mode='mesh_v3'", [network.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(json, text)| {
        Ok((
            serde_json::from_str(&json)
                .map_err(|_| Error::Configuration("invalid persisted v3 source"))?,
            text,
        ))
    })
    .transpose()
}

pub(super) fn active(db: &Connection, network: &NetworkId) -> Result<bool, Error> {
    Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM network_policy_sources WHERE network_id=?1 AND source_mode='mesh_v3')",[network.as_str()],|r|r.get(0))?)
}

fn snapshot(db: &Connection, network: &NetworkId) -> Result<NetworkPolicySnapshot, Error> {
    let revision = configuration::revision(db, network)?;
    if let Some((policy, source_jsonc)) = source(db, network)? {
        let publication = super::publication::mesh_v3::Publication::read(db, network, &policy)?;
        return Ok(NetworkPolicySnapshot {
            network_id: network.as_str().to_owned(),
            revision,
            source_mode: PolicySourceMode::MeshV3,
            policy: Some(policy),
            source_jsonc,
            valid: publication.valid(),
            diagnostics: publication.diagnostics(),
            migration_proposal: None,
        });
    }
    let access = v3::migrate_access(&super::access::source(db, network)?);
    let routing = v3::migrate_routing(&configuration::source_policy(db, network)?);
    let mut diagnostics = access.diagnostics;
    diagnostics.extend(routing.diagnostics);
    // Existing explicit candidate graph remains the physical constraint. Default
    // allow admits only those candidates, not unconfigured dial pairs.
    let migration_proposal = match (access.converted, routing.converted) {
        (Some(grants), Some(routes)) if routes.exits.is_empty() && grants.is_empty() => {
            Some(NetworkPolicy {
                grants,
                mesh: routes.mesh,
                ..NetworkPolicy::default_template()
            })
        }
        (Some(_), Some(routes)) if routes.exits.is_empty() => {
            // Symbolic selector conversion alone does not prove path equality:
            // v2 direct/exact-hop routing cannot generally express v3 automatic
            // candidate-graph planning between mandatory waypoints, including
            // future inventory changes. Never label that draft equivalent.
            diagnostics.push(v3::Diagnostic {field:"migration.routing".into(),message:"Legacy executable direct/exact-hop routing is not proven equivalent to v3 automatic Mesh planning; retain v2 or explicitly replace the source after reviewing grants and mandatory waypoints".into()});
            None
        }
        (_, Some(routes)) if !routes.exits.is_empty() => {
            diagnostics.push(v3::Diagnostic {
                field: "routing".into(),
                message: "External routes require P3c; legacy source retained".into(),
            });
            None
        }
        _ => None,
    };
    Ok(NetworkPolicySnapshot {
        network_id: network.as_str().to_owned(),
        revision,
        source_mode: PolicySourceMode::LegacyV2,
        policy: None,
        source_jsonc: None,
        valid: false,
        diagnostics,
        migration_proposal,
    })
}

pub(super) fn check_epoch(
    db: &Connection,
    network: &NetworkId,
    id: &str,
    request: &MeshExecutionRequest,
) -> Result<(), Error> {
    if request.epoch == 0 || !local::valid_credential(&request.instance_id) {
        return Err(Error::Invalid("invalid execution runtime binding"));
    }
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM connection_runtime_leases WHERE network_id=?1 AND node_id=?2 AND epoch=?3 AND instance_id=?4)",params![network.as_str(),id,request.epoch,request.instance_id],|r|r.get::<_,bool>(0))? {
        return Err(Error::Conflict);
    }
    Ok(())
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
            .map_err(|_| Error::Invalid("cannot encode v3 source"))?
            .len()
            > MAX_SOURCE_JSONC_BYTES
            || request
                .source_jsonc
                .as_ref()
                .is_some_and(|s| s.len() > MAX_SOURCE_JSONC_BYTES)
        {
            return Err(Error::Unprocessable("v3 source exceeds 256 KiB"));
        }
        if let Some(text) = &request.source_jsonc {
            let parsed: NetworkPolicy = super::jsonc::parse_source(text)?;
            if parsed != request.policy {
                return Err(Error::Unprocessable("source_jsonc differs from policy"));
            }
        }
        // Fail closed on unimplemented external source semantics, including an
        // assignment whose fallback could otherwise be silently discarded.
        if !request.policy.public_rules.is_empty()
            || !request.policy.profiles.is_empty()
            || !request.policy.assignments.is_empty()
            || request.policy.automatic != v3::AutomaticRouting::default()
            || request.policy.grants.iter().flat_map(|g| &g.dst).any(|s| {
                s == "autogroup:internet" || s.starts_with("ip:") || s.starts_with("domain:")
            })
        {
            return Err(Error::Unprocessable(
                "Mesh execution supports only network connections, grants and Mesh routes; external profiles require P3c",
            ));
        }
        let tx = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&tx, network)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        let publication =
            super::publication::mesh_v3::Publication::read(&tx, network, &request.policy)?;
        if !publication.valid() {
            return Err(Error::Unprocessable(
                "invalid v3 source; retain legacy source until references and policy are corrected",
            ));
        }
        let old = source(&tx, network)?;
        if old.as_ref() != Some(&(request.policy.clone(), request.source_jsonc.clone())) {
            let json = serde_json::to_string(&request.policy)
                .map_err(|_| Error::Configuration("cannot encode v3 source"))?;
            tx.execute("INSERT INTO network_policy_sources(network_id,source_mode,policy,source_jsonc) VALUES(?1,'mesh_v3',?2,?3) ON CONFLICT(network_id) DO UPDATE SET source_mode=excluded.source_mode,policy=excluded.policy,source_jsonc=excluded.source_jsonc",params![network.as_str(),json,request.source_jsonc])?;
            configuration::bump(&tx, network)?;
            super::publication::finish_change(
                &tx,
                network,
                super::publication::ChangeKind::Adjacency,
            )?;
        }
        let result = snapshot(&tx, network)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn mesh_execution(
        &mut self,
        token: &str,
        request: &MeshExecutionRequest,
    ) -> Result<MeshExecutionResponse, Error> {
        let network = self.node_network(token)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = super::connection_runtime::bound_node(&tx, &network, token)?;
        check_epoch(&tx, &network, &id, request)?;
        if request.capabilities != [MeshExecutionCapability::Ipv4ExactPathV1] {
            return Err(Error::Unprocessable(
                "IPv4 exact-path execution v1 capability required",
            ));
        }
        let changed= !tx.query_row("SELECT EXISTS(SELECT 1 FROM mesh_execution_capabilities WHERE network_id=?1 AND node_id=?2 AND epoch=?3 AND instance_id=?4)",params![network.as_str(),id,request.epoch,request.instance_id],|r|r.get::<_,bool>(0))?;
        if changed {
            tx.execute("INSERT INTO mesh_execution_capabilities(node_id,network_id,epoch,instance_id) VALUES(?1,?2,?3,?4) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,instance_id=excluded.instance_id WHERE network_id=excluded.network_id",params![id,network.as_str(),request.epoch,request.instance_id])?;
            if active(&tx, &network)? {
                configuration::bump(&tx, &network)?;
                super::publication::finish_change(
                    &tx,
                    &network,
                    super::publication::ChangeKind::Adjacency,
                )?;
            }
        }
        let revision = configuration::revision(&tx, &network)?;
        let response = if let Some((policy, _)) = source(&tx, &network)? {
            let snapshot = super::publication::mesh_v3::Publication::read(&tx, &network, &policy)?
                .project(&id, &self.material_key)?;
            MeshExecutionResponse {
                source_mode: PolicySourceMode::MeshV3,
                revision,
                snapshot: Some(snapshot),
            }
        } else {
            MeshExecutionResponse {
                source_mode: PolicySourceMode::LegacyV2,
                revision,
                snapshot: None,
            }
        };
        tx.commit()?;
        Ok(response)
    }

    pub fn acknowledge_mesh_execution(
        &mut self,
        token: &str,
        request: &AcknowledgeMeshExecutionRequest,
    ) -> Result<AcknowledgeMeshExecutionResponse, Error> {
        let network = self.node_network(token)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = super::connection_runtime::bound_node(&tx, &network, token)?;
        check_epoch(&tx, &network, &id, &request.runtime)?;
        if request.runtime.capabilities != [MeshExecutionCapability::Ipv4ExactPathV1] {
            return Err(Error::Conflict);
        }
        let (policy, _) = source(&tx, &network)?.ok_or(Error::Conflict)?;
        let current = super::publication::mesh_v3::Publication::read(&tx, &network, &policy)?
            .project(&id, &self.material_key)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Conflict);
        }
        tx.execute("INSERT INTO mesh_execution_receipts(node_id,network_id,epoch,revision,digest) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,revision=excluded.revision,digest=excluded.digest WHERE network_id=excluded.network_id",params![id,network.as_str(),request.runtime.epoch,request.revision,request.digest])?;
        tx.commit()?;
        Ok(AcknowledgeMeshExecutionResponse {
            revision: request.revision,
            digest: request.digest.clone(),
            saved: true,
        })
    }
}

#[cfg(test)]
mod tests;
