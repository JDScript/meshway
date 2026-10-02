use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::*;
use rusqlite::{Connection, TransactionBehavior, params};

use super::{NetworkId, Store, configuration, ipam, jsonc, publication, tags};
use crate::Error;

#[cfg(test)]
mod tests;

fn valid_legacy_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
}

fn migrated_id(ids: impl Iterator<Item = String>, source: &str, destination: &str) -> String {
    ids.filter(|id| valid_legacy_id(id))
        .min()
        .unwrap_or_else(|| {
            local::digest(&format!("legacy:{source}:{destination}"))
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        })
}

fn migrate_legacy(legacy: MeshPathAuthorization) -> MeshAccessPolicy {
    let mut directed: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for grant in legacy.grants {
        if let (Some(source), Some(destination)) = (grant.path.first(), grant.path.last())
            && source != destination
        {
            directed
                .entry((source.clone(), destination.clone()))
                .or_default()
                .push(grant.id);
        }
    }
    let mut rules = Vec::new();
    while let Some(((source, destination), mut ids)) = directed.pop_first() {
        let reverse = directed.remove(&(destination.clone(), source.clone()));
        let bidirectional = reverse.is_some();
        if let Some(reverse) = reverse {
            ids.extend(reverse);
        }
        rules.push(MeshAccessRule {
            id: migrated_id(ids.into_iter(), &source, &destination),
            source: vec![format!("node:{source}")],
            destination: vec![format!("node:{destination}")],
            action: AccessAction::Allow,
            bidirectional,
        });
    }
    MeshAccessPolicy {
        default_action: AccessAction::Reject,
        rules,
    }
}

pub(super) fn upgrade(connection: &mut Connection) -> Result<(), Error> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if (6..=8).contains(&version) {
        return Ok(());
    }
    if version == 4 {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE mesh_access_source (
                id INTEGER PRIMARY KEY CHECK (id = 1), policy TEXT NOT NULL
             );
             INSERT INTO mesh_access_source VALUES (1, '{\"grants\":[]}');
             PRAGMA user_version = 5;",
        )?;
        transaction.commit()?;
    } else if version != 5 {
        return Err(Error::Configuration(
            "access schema upgrade requires schema 4 or 5",
        ));
    }

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let legacy_json: String = transaction.query_row(
        "SELECT policy FROM mesh_access_source WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    let legacy: MeshPathAuthorization = serde_json::from_str(&legacy_json)
        .map_err(|_| Error::Configuration("invalid legacy mesh access"))?;
    let revision_must_change = !legacy.grants.is_empty();
    let policy = serde_json::to_string(&migrate_legacy(legacy))
        .map_err(|_| Error::Configuration("cannot migrate mesh access"))?;
    if transaction.query_row("SELECT revision FROM configuration WHERE id=1", [], |row| {
        row.get::<_, u32>(0)
    })? == u32::MAX
    {
        return Err(Error::Capacity("configuration revision exhausted"));
    }
    transaction.execute_batch(
        "ALTER TABLE mesh_access_source RENAME TO mesh_access_legacy_v5;
         CREATE TABLE mesh_access_source (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            policy TEXT NOT NULL,
            source_jsonc TEXT
         );
         ALTER TABLE configuration ADD COLUMN source_jsonc TEXT;
         CREATE TABLE tag_definitions (
            tag TEXT PRIMARY KEY,
            description TEXT NOT NULL
         );
         CREATE TABLE node_tags (
            node_id TEXT NOT NULL REFERENCES nodes(id),
            tag TEXT NOT NULL REFERENCES tag_definitions(tag),
            PRIMARY KEY(node_id, tag)
         );",
    )?;
    transaction.execute(
        "INSERT INTO mesh_access_source(id,policy,source_jsonc) VALUES (1,?1,NULL)",
        [policy],
    )?;
    if revision_must_change {
        transaction.execute(
            "UPDATE configuration SET revision=revision+1 WHERE id=1",
            [],
        )?;
    }
    transaction.pragma_update(None, "user_version", 6)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn source(
    connection: &Connection,
    scope: &NetworkId,
) -> Result<MeshAccessPolicy, Error> {
    let json: String = connection.query_row(
        "SELECT policy FROM mesh_access_source WHERE id = 1 AND network_id=?1",
        [scope.as_str()],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid stored mesh access"))
}

fn source_jsonc(connection: &Connection, scope: &NetworkId) -> Result<Option<String>, Error> {
    Ok(connection.query_row(
        "SELECT source_jsonc FROM mesh_access_source WHERE id=1 AND network_id=?1",
        [scope.as_str()],
        |row| row.get(0),
    )?)
}

pub(super) fn bindings(
    connection: &Connection,
    scope: &NetworkId,
) -> Result<BTreeMap<String, String>, Error> {
    Ok(connection.prepare(
        "SELECT node_id, public_key FROM connection_identities JOIN nodes ON nodes.id = node_id AND nodes.network_id=connection_identities.network_id WHERE nodes.revoked_at IS NULL AND nodes.network_id=?1 ORDER BY node_id",
    )?.query_map([scope.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?)
}

fn validate(
    policy: &MeshAccessPolicy,
    nodes: &[MeshNode],
    catalog: &BTreeSet<String>,
) -> Vec<PolicyDiagnostic> {
    meshway_policy::validate_access_policy(policy, nodes, catalog)
}

fn validate_current(
    connection: &Connection,
    scope: &NetworkId,
    policy: &MeshAccessPolicy,
) -> Result<Vec<PolicyDiagnostic>, Error> {
    Ok(validate(
        policy,
        &configuration::active_nodes(connection, scope)?,
        &tags::catalog(connection, scope)?,
    ))
}

fn admin_snapshot(connection: &Connection, scope: &NetworkId) -> Result<MeshAccessSnapshot, Error> {
    let policy = source(connection, scope)?;
    let diagnostics = validate_current(connection, scope, &policy)?;
    Ok(MeshAccessSnapshot {
        revision: configuration::revision(connection, scope)?,
        policy,
        source_jsonc: source_jsonc(connection, scope)?,
        valid: diagnostics.is_empty(),
        diagnostics,
    })
}

fn compiled_authorizations(
    connection: &Connection,
    scope: &NetworkId,
) -> Result<(bool, BTreeMap<String, MeshPathAuthorization>), Error> {
    Ok(publication::Publication::read(connection, scope)?.into_authorizations())
}

#[cfg(test)]
fn compiled_authorizations_with(
    connection: &Connection,
    scope: &NetworkId,
    route_evaluated: impl FnMut(),
) -> Result<(bool, BTreeMap<String, MeshPathAuthorization>), Error> {
    Ok(
        publication::Publication::read_with(connection, scope, route_evaluated)?
            .into_authorizations(),
    )
}

impl Store {
    pub fn mesh_access(&self, scope: &NetworkId) -> Result<MeshAccessSnapshot, Error> {
        admin_snapshot(&self.connection, scope)
    }

    pub fn save_mesh_access(
        &mut self,
        scope: &NetworkId,
        request: &SaveMeshAccessRequest,
    ) -> Result<MeshAccessSnapshot, Error> {
        if request
            .source_jsonc
            .as_ref()
            .is_some_and(|source| source.len() > MAX_SOURCE_JSONC_BYTES)
        {
            return Err(Error::Unprocessable("source_jsonc exceeds 256 KiB"));
        }
        if let Some(source) = &request.source_jsonc {
            let parsed: MeshAccessPolicy = jsonc::parse_source(source)?;
            if parsed != request.policy {
                return Err(Error::Unprocessable(
                    "source_jsonc policy does not equal request.policy",
                ));
            }
        }
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Immediate,
        )?;
        if configuration::revision(&transaction, scope)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        if !validate_current(&transaction, scope, &request.policy)?.is_empty() {
            return Err(Error::Unprocessable(
                "invalid mesh access policy; inspect selectors, references and ordered rules",
            ));
        }
        let changed = source(&transaction, scope)? != request.policy
            || source_jsonc(&transaction, scope)? != request.source_jsonc;
        if changed {
            let encoded = serde_json::to_string(&request.policy)
                .map_err(|_| Error::Configuration("cannot encode mesh access"))?;
            transaction.execute(
                "UPDATE mesh_access_source SET policy=?1, source_jsonc=?2 WHERE id=1 AND network_id=?3",
                params![encoded, request.source_jsonc, scope.as_str()],
            )?;
            configuration::bump(&transaction, scope)?;
            publication::finish_change(&transaction, scope, publication::ChangeKind::Adjacency)?;
        }
        let response = admin_snapshot(&transaction, scope)?;
        transaction.commit()?;
        Ok(response)
    }

    pub fn preview_flow(
        &self,
        scope: &NetworkId,
        request: &PreviewFlowRequest,
    ) -> Result<PreviewFlowResponse, Error> {
        let nodes = configuration::active_nodes(&self.connection, scope)?;
        let access = admin_snapshot(&self.connection, scope)?;
        let routing_policy = configuration::source_policy(&self.connection, scope)?;
        let network = ipam::network(&self.connection, scope)?;
        let routing = meshway_policy::preview(&routing_policy, &network, &nodes, &request.flow);
        let source_active = nodes
            .iter()
            .any(|node| node.id == request.flow.source_node_id);
        let explicit_destination_invalid = request
            .flow
            .destination_node_id
            .as_ref()
            .is_some_and(|id| !nodes.iter().any(|node| node.id == id.as_str()));
        let destination = request
            .flow
            .destination_node_id
            .as_deref()
            .filter(|id| nodes.iter().any(|node| node.id == *id))
            .or_else(|| {
                request.flow.destination_ip.as_deref().and_then(|ip| {
                    nodes
                        .iter()
                        .find(|node| node.mesh_ipv4 == ip)
                        .map(|node| node.id.as_str())
                })
            });
        let authorization = if !access.valid {
            FlowAuthorizationDecision {
                action: FlowAuthorizationAction::Invalid,
                rule_id: None,
                matched_direction: None,
                reason: "Saved access policy is invalid".into(),
            }
        } else if !source_active || explicit_destination_invalid {
            FlowAuthorizationDecision {
                action: FlowAuthorizationAction::Invalid,
                rule_id: None,
                matched_direction: None,
                reason: "Flow references a node outside the active network".into(),
            }
        } else if let Some(destination) = destination {
            meshway_policy::evaluate_access(
                &access.policy,
                &request.flow.source_node_id,
                destination,
                &tags::assignments(&self.connection, scope)?,
            )
        } else {
            FlowAuthorizationDecision {
                action: FlowAuthorizationAction::Reject,
                rule_id: None,
                matched_direction: None,
                reason: "Endpoint authorization requires a resolved mesh destination".into(),
            }
        };
        let execution_supported = access.valid
            && authorization.action == FlowAuthorizationAction::Allow
            && routing.valid
            && routing.decision.action == RouteDecisionAction::Route
            && routing.decision.exit_node_id.is_none()
            && request.flow.domain.is_none()
            && request.flow.destination_country.is_none()
            && request
                .flow
                .destination_ip
                .as_deref()
                .is_none_or(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
            && destination.is_some()
            && compiled_authorizations(&self.connection, scope)?
                .1
                .get(&request.flow.source_node_id)
                .is_some_and(|authorization| {
                    authorization
                        .grants
                        .iter()
                        .any(|grant| grant.path == routing.decision.path)
                });
        Ok(PreviewFlowResponse {
            revision: access.revision,
            valid: access.valid && routing.valid,
            authorization,
            routing: routing.decision,
            diagnostics: access
                .diagnostics
                .into_iter()
                .chain(routing.diagnostics)
                .collect(),
            execution_supported,
            execution_reason: if execution_supported {
                "Saved ACL and routing resolve to a runtime-eligible exact IPv4 mesh path".into()
            } else {
                "Flow is not an authorized runtime-eligible exact IPv4 mesh path with known keys"
                    .into()
            },
        })
    }

    pub fn agent_dataplane(&mut self, token: &str) -> Result<AgentDataplaneSnapshot, Error> {
        let scope = self.node_network(token)?;
        let transaction = super::account_authorization::transaction(
            &mut self.connection,
            TransactionBehavior::Deferred,
        )?;
        let node_id = configuration::node_id(&transaction, &scope, token)?;
        let snapshot = publication::Publication::read(&transaction, &scope)?.dataplane(&node_id)?;
        transaction.commit()?;
        Ok(snapshot)
    }
}
