use std::collections::{BTreeMap, BTreeSet};

use meshway_protocol::*;
use rusqlite::{Connection, TransactionBehavior, params};

use super::{Store, configuration, connections, ipam, jsonc, tags};
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
    if (6..=7).contains(&version) {
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
    if configuration::revision(&transaction)? == u32::MAX {
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
        configuration::bump(&transaction)?;
    }
    ensure_publishable(&transaction)?;
    transaction.pragma_update(None, "user_version", 6)?;
    transaction.commit()?;
    Ok(())
}

pub(super) fn source(connection: &Connection) -> Result<MeshAccessPolicy, Error> {
    let json: String = connection.query_row(
        "SELECT policy FROM mesh_access_source WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid stored mesh access"))
}

fn source_jsonc(connection: &Connection) -> Result<Option<String>, Error> {
    Ok(connection.query_row(
        "SELECT source_jsonc FROM mesh_access_source WHERE id=1",
        [],
        |row| row.get(0),
    )?)
}

fn bindings(connection: &Connection) -> Result<BTreeMap<String, String>, Error> {
    Ok(connection.prepare(
        "SELECT node_id, public_key FROM connection_identities JOIN nodes ON nodes.id = node_id WHERE nodes.revoked_at IS NULL ORDER BY node_id",
    )?.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?.collect::<Result<_, _>>()?)
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
    policy: &MeshAccessPolicy,
) -> Result<Vec<PolicyDiagnostic>, Error> {
    Ok(validate(
        policy,
        &configuration::active_nodes(connection)?,
        &tags::catalog(connection)?,
    ))
}

fn admin_snapshot(connection: &Connection) -> Result<MeshAccessSnapshot, Error> {
    let policy = source(connection)?;
    let diagnostics = validate_current(connection, &policy)?;
    Ok(MeshAccessSnapshot {
        revision: configuration::revision(connection)?,
        policy,
        source_jsonc: source_jsonc(connection)?,
        valid: diagnostics.is_empty(),
        diagnostics,
    })
}

fn authorization_id(path: &[String]) -> String {
    local::digest(&format!("effective:{}", path.join(":")))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn compiled_authorizations(
    connection: &Connection,
) -> Result<(bool, BTreeMap<String, MeshPathAuthorization>), Error> {
    compiled_authorizations_with(connection, || {})
}

fn compiled_authorizations_with(
    connection: &Connection,
    mut route_evaluated: impl FnMut(),
) -> Result<(bool, BTreeMap<String, MeshPathAuthorization>), Error> {
    let nodes = configuration::active_nodes(connection)?;
    let mut result: BTreeMap<_, _> = nodes
        .iter()
        .map(|node| (node.id.clone(), MeshPathAuthorization::default()))
        .collect();
    let access = source(connection)?;
    let catalog = tags::catalog(connection)?;
    if !validate(&access, &nodes, &catalog).is_empty() {
        return Ok((false, result));
    }
    if access.default_action == AccessAction::Reject
        && access
            .rules
            .iter()
            .all(|rule| rule.action == AccessAction::Reject)
    {
        return Ok((true, result));
    }
    let keys: BTreeMap<_, _> = bindings(connection)?
        .into_iter()
        .filter(|(_, key)| valid_connection_public_key(key))
        .collect();
    let eligible: Vec<_> = nodes
        .iter()
        .filter(|node| keys.contains_key(&node.id))
        .collect();
    if eligible.len() < 2 {
        return Ok((true, result));
    }
    let routing = configuration::source_policy(connection)?;
    let network = ipam::network(connection)?;
    let prepared = meshway_policy::prepare_routing(&routing, &network, &nodes);
    if !prepared.report().is_valid() {
        return Ok((true, result));
    }
    let assignments = tags::assignments(connection)?;
    let eligible_ids: BTreeSet<_> = eligible.iter().map(|node| node.id.as_str()).collect();
    let mut connection_source = None;
    let mut identity_unions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for source in &eligible {
        for destination in &eligible {
            if source.id == destination.id
                || meshway_policy::evaluate_access(
                    &access,
                    &source.id,
                    &destination.id,
                    &assignments,
                )
                .action
                    != FlowAuthorizationAction::Allow
            {
                continue;
            }
            route_evaluated();
            let Some(path) = prepared.compile_mesh_endpoint(&source.id, &destination.id) else {
                continue;
            };
            if path.first() != Some(&source.id)
                || path.last() != Some(&destination.id)
                || path.iter().any(|id| !eligible_ids.contains(id.as_str()))
            {
                continue;
            }
            if path.len() > MAX_MESH_ACCESS_PATH_NODES {
                return Err(Error::Capacity(
                    "authorized route exceeds the 8-node exact-path limit",
                ));
            }
            let grant = MeshAccessGrant {
                id: authorization_id(&path),
                path,
            };
            for participant in &grant.path {
                let authorization = result.get(participant).ok_or(Error::Configuration(
                    "routing returned an inactive path node",
                ))?;
                if authorization.grants.len() == MAX_MESH_ACCESS_GRANTS {
                    return Err(Error::Capacity(
                        "effective access exceeds 128 exact paths for a node",
                    ));
                }
                if !identity_unions.contains_key(participant) {
                    let source = if let Some(source) = &connection_source {
                        source
                    } else {
                        connection_source.insert(connections::admin_snapshot(connection)?)
                    };
                    let adjacency = connections::projection(source, participant)?.payload;
                    identity_unions.insert(
                        participant.clone(),
                        adjacency
                            .peers
                            .into_iter()
                            .map(|peer| peer.node_id)
                            .collect(),
                    );
                }
                let union = identity_unions
                    .get_mut(participant)
                    .expect("inserted participant identity union");
                union.extend(grant.path.iter().filter(|id| *id != participant).cloned());
                if union.len() > MAX_DATAPLANE_IDENTITIES {
                    return Err(Error::Capacity(
                        "dataplane exceeds 64 union peer identities",
                    ));
                }
            }
            for participant in &grant.path {
                result
                    .get_mut(participant)
                    .expect("validated path participant")
                    .grants
                    .push(grant.clone());
            }
        }
    }
    Ok((true, result))
}

fn identities(
    id: &str,
    connections: &NodeConnections,
    access: &MeshPathAuthorization,
    keys: &BTreeMap<String, String>,
) -> Result<Vec<ConnectionPeer>, Error> {
    let ids: BTreeSet<_> = connections
        .peers
        .iter()
        .map(|peer| peer.node_id.as_str())
        .chain(
            access
                .grants
                .iter()
                .flat_map(|grant| grant.path.iter().map(String::as_str)),
        )
        .filter(|peer| *peer != id)
        .collect();
    if ids.len() > MAX_DATAPLANE_IDENTITIES {
        return Err(Error::Capacity(
            "dataplane exceeds 64 union peer identities",
        ));
    }
    ids.into_iter()
        .map(|id| {
            let key = keys
                .get(id)
                .filter(|key| valid_connection_public_key(key))
                .ok_or(Error::Configuration(
                    "dataplane peer has no canonical immutable binding",
                ))?;
            Ok(ConnectionPeer {
                node_id: id.to_owned(),
                public_key: key.clone(),
            })
        })
        .collect()
}

pub(super) fn ensure_publishable(connection: &Connection) -> Result<(), Error> {
    compiled_authorizations(connection)?;
    Ok(())
}

impl Store {
    pub fn mesh_access(&self) -> Result<MeshAccessSnapshot, Error> {
        admin_snapshot(&self.connection)
    }

    pub fn save_mesh_access(
        &mut self,
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if configuration::revision(&transaction)? != request.expected_revision {
            return Err(Error::Conflict);
        }
        if !validate_current(&transaction, &request.policy)?.is_empty() {
            return Err(Error::Unprocessable(
                "invalid mesh access policy; inspect selectors, references and ordered rules",
            ));
        }
        let changed = source(&transaction)? != request.policy
            || source_jsonc(&transaction)? != request.source_jsonc;
        if changed {
            let encoded = serde_json::to_string(&request.policy)
                .map_err(|_| Error::Configuration("cannot encode mesh access"))?;
            transaction.execute(
                "UPDATE mesh_access_source SET policy=?1, source_jsonc=?2 WHERE id=1",
                params![encoded, request.source_jsonc],
            )?;
            configuration::bump(&transaction)?;
            configuration::ensure_publishable(&transaction)?;
            connections::ensure_publishable(&transaction)?;
            ensure_publishable(&transaction)?;
        }
        let response = admin_snapshot(&transaction)?;
        transaction.commit()?;
        Ok(response)
    }

    pub fn preview_flow(&self, request: &PreviewFlowRequest) -> Result<PreviewFlowResponse, Error> {
        let nodes = configuration::active_nodes(&self.connection)?;
        let access = admin_snapshot(&self.connection)?;
        let routing_policy = configuration::source_policy(&self.connection)?;
        let network = ipam::network(&self.connection)?;
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
                &tags::assignments(&self.connection)?,
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
            && compiled_authorizations(&self.connection)?
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
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let node_id = configuration::node_id(&transaction, token)?;
        let configuration = configuration::snapshot(&transaction, &node_id)?.payload;
        let connections =
            connections::projection(&connections::admin_snapshot(&transaction)?, &node_id)?.payload;
        let (access_valid, mut authorizations) = compiled_authorizations(&transaction)?;
        let access_policy = authorizations.remove(&node_id).unwrap_or_default();
        let identities = identities(
            &node_id,
            &connections,
            &access_policy,
            &bindings(&transaction)?,
        )?;
        let revision = configuration.revision;
        if connections.revision != revision {
            return Err(Error::Configuration("incoherent dataplane revision"));
        }
        let payload = NodeDataplane {
            schema_version: DATAPLANE_SCHEMA_VERSION,
            revision,
            node_id,
            configuration,
            connections,
            access_policy,
            access_valid,
            identities,
        };
        let digest = dataplane_digest(&payload)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?;
        let snapshot = AgentDataplaneSnapshot { digest, payload };
        if serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode dataplane"))?
            .len()
            > MAX_DATAPLANE_BYTES
        {
            return Err(Error::Capacity(
                "dataplane exceeds 2 MiB publication budget",
            ));
        }
        transaction.commit()?;
        Ok(snapshot)
    }
}
