use meshway_protocol::{
    AcknowledgeConfigurationRequest, AcknowledgeConfigurationResponse,
    CONFIGURATION_SCHEMA_VERSION, ConfigurationSnapshot, MAX_CONFIGURATION_BYTES, MeshNode,
    NodeConfiguration, NodeSummary, PolicyDiagnostic, PolicySnapshot, PreviewPolicyRequest,
    PreviewPolicyResponse, RoutingPolicy, SavePolicyRequest, SetNodeCountryRequest,
    configuration_digest, local::digest,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{Store, access, find_node, ipam, jsonc};
use crate::Error;

const DIAGNOSTIC_BUDGET: usize = 32 * 1024;

/// Diagnostic references can repeat a long user-supplied rule ID. Keep reports
/// useful while bounding their serialized size independently of policy size.
fn bounded_diagnostics(diagnostics: Vec<PolicyDiagnostic>) -> Vec<PolicyDiagnostic> {
    let mut bounded = Vec::new();
    let mut remaining = DIAGNOSTIC_BUDGET - 512;
    for diagnostic in diagnostics {
        let size = serde_json::to_vec(&diagnostic).map_or(usize::MAX, |bytes| bytes.len() + 1);
        if size > remaining || bounded.len() == 63 {
            bounded.push(PolicyDiagnostic {
                rule_id: None,
                field: "policy".into(),
                message: "Additional diagnostics omitted by the bounded publication budget".into(),
            });
            break;
        }
        remaining -= size;
        bounded.push(diagnostic);
    }
    bounded
}

pub(super) fn revision(connection: &Connection) -> Result<u32, Error> {
    Ok(connection.query_row(
        "SELECT revision FROM configuration WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

pub(super) fn bump(connection: &Connection) -> Result<(), Error> {
    if revision(connection)? == u32::MAX {
        return Err(Error::Capacity("configuration revision exhausted"));
    }
    connection.execute(
        "UPDATE configuration SET revision = revision + 1 WHERE id = 1",
        [],
    )?;
    Ok(())
}

fn compare_revision(connection: &Connection, expected: u32) -> Result<(), Error> {
    if revision(connection)? != expected {
        return Err(Error::Conflict);
    }
    Ok(())
}

pub(super) fn active_nodes(connection: &Connection) -> Result<Vec<MeshNode>, Error> {
    Ok(connection.prepare("SELECT id, name, country, mesh_ipv4, mesh_ipv6 FROM nodes WHERE revoked_at IS NULL ORDER BY id")?
        .query_map([], |row| Ok(MeshNode { id: row.get(0)?, name: row.get(1)?, country: row.get(2)?, mesh_ipv4: row.get(3)?, mesh_ipv6: row.get(4)? }))?
        .collect::<Result<_, _>>()?)
}

pub(super) fn source_policy(connection: &Connection) -> Result<RoutingPolicy, Error> {
    let json: String =
        connection.query_row("SELECT policy FROM configuration WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    serde_json::from_str(&json).map_err(|_| Error::Configuration("invalid persisted policy"))
}

fn source_jsonc(connection: &Connection) -> Result<Option<String>, Error> {
    Ok(connection.query_row(
        "SELECT source_jsonc FROM configuration WHERE id=1",
        [],
        |row| row.get(0),
    )?)
}

fn policy_snapshot(connection: &Connection) -> Result<PolicySnapshot, Error> {
    let policy = source_policy(connection)?;
    let network = ipam::network(connection)?;
    let report = meshway_policy::compile(&policy, &network, &active_nodes(connection)?);
    Ok(PolicySnapshot {
        revision: revision(connection)?,
        network,
        policy,
        source_jsonc: source_jsonc(connection)?,
        valid: report.is_valid(),
        diagnostics: bounded_diagnostics(report.diagnostics),
    })
}

pub(super) fn node_id(connection: &Connection, token: &str) -> Result<String, Error> {
    let (id, revoked): (String, bool) = connection
        .query_row(
            "SELECT id, revoked_at IS NOT NULL FROM nodes WHERE credential_hash = ?1",
            [digest(token)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(Error::Unauthorized)?;
    if revoked {
        return Err(Error::Forbidden);
    }
    Ok(id)
}

pub(super) fn snapshot(connection: &Connection, id: &str) -> Result<ConfigurationSnapshot, Error> {
    let mut nodes = active_nodes(connection)?;
    let network = ipam::network(connection)?;
    let policy = source_policy(connection)?;
    let report = meshway_policy::compile(&policy, &network, &nodes);
    let node_index = nodes
        .iter()
        .position(|node| node.id == id)
        .ok_or(Error::Forbidden)?;
    let node = nodes.remove(node_index);
    let valid = report.is_valid();
    let payload = NodeConfiguration {
        schema_version: CONFIGURATION_SCHEMA_VERSION,
        revision: revision(connection)?,
        network,
        node,
        peers: nodes,
        routing_policy: if valid {
            report.normalized_policy
        } else {
            RoutingPolicy::default()
        },
        policy_valid: valid,
        diagnostics: bounded_diagnostics(report.diagnostics),
    };
    let digest = configuration_digest(&payload)
        .map_err(|_| Error::Configuration("cannot encode configuration"))?;
    Ok(ConfigurationSnapshot { digest, payload })
}

/// Every active node appears exactly once (self or peers). Moving an object
/// between those positions preserves the encoded size, so one snapshot suffices.
pub(super) fn ensure_publishable(connection: &Connection) -> Result<(), Error> {
    let id: Option<String> = connection
        .query_row(
            "SELECT id FROM nodes WHERE revoked_at IS NULL ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = id {
        let mut snapshot = snapshot(connection, &id)?;
        // Always reserve room for a future fail-closed report. Withdrawing a
        // revoked peer can never make this base projection larger.
        snapshot.payload.diagnostics.clear();
        let bytes = serde_json::to_vec(&snapshot)
            .map_err(|_| Error::Configuration("cannot encode configuration"))?;
        if bytes.len() > MAX_CONFIGURATION_BYTES - DIAGNOSTIC_BUDGET {
            return Err(Error::Capacity(
                "configuration exceeds 1 MiB publication budget",
            ));
        }
    }
    Ok(())
}

impl Store {
    pub fn revision(&self) -> Result<u32, Error> {
        revision(&self.connection)
    }

    pub fn policy(&self) -> Result<PolicySnapshot, Error> {
        policy_snapshot(&self.connection)
    }

    pub fn preview(&self, request: &PreviewPolicyRequest) -> Result<PreviewPolicyResponse, Error> {
        let result = meshway_policy::preview(
            &request.policy,
            &ipam::network(&self.connection)?,
            &active_nodes(&self.connection)?,
            &request.flow,
        );
        Ok(PreviewPolicyResponse {
            revision: self.revision()?,
            valid: result.valid,
            diagnostics: bounded_diagnostics(result.diagnostics),
            decision: result.decision,
        })
    }

    pub fn save_policy(&mut self, request: &SavePolicyRequest) -> Result<PolicySnapshot, Error> {
        if request
            .source_jsonc
            .as_ref()
            .is_some_and(|source| source.len() > meshway_protocol::MAX_SOURCE_JSONC_BYTES)
        {
            return Err(Error::Unprocessable("source_jsonc exceeds 256 KiB"));
        }
        if let Some(source) = &request.source_jsonc {
            let parsed: RoutingPolicy = jsonc::parse_source(source)?;
            if parsed != request.policy {
                return Err(Error::Unprocessable(
                    "source_jsonc policy does not equal request.policy",
                ));
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        compare_revision(&transaction, request.expected_revision)?;
        let report = meshway_policy::compile(
            &request.policy,
            &ipam::network(&transaction)?,
            &active_nodes(&transaction)?,
        );
        if !report.is_valid() {
            return Err(Error::Unprocessable(
                "policy is invalid; preview it for diagnostics",
            ));
        }
        let stored_policy = if request.source_jsonc.is_some() {
            &request.policy
        } else {
            &report.normalized_policy
        };
        if source_policy(&transaction)? != *stored_policy
            || source_jsonc(&transaction)? != request.source_jsonc
        {
            let json = serde_json::to_string(stored_policy)
                .map_err(|_| Error::Configuration("cannot encode policy"))?;
            transaction.execute(
                "UPDATE configuration SET policy=?1, source_jsonc=?2 WHERE id=1",
                params![json, request.source_jsonc],
            )?;
            bump(&transaction)?;
            ensure_publishable(&transaction)?;
            access::ensure_publishable(&transaction)?;
        }
        let snapshot = policy_snapshot(&transaction)?;
        transaction.commit()?;
        Ok(snapshot)
    }

    pub fn set_country(&mut self, request: &SetNodeCountryRequest) -> Result<NodeSummary, Error> {
        let country = request
            .country
            .as_ref()
            .map(|country| {
                if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                    return Err(Error::Invalid("country must be two ASCII letters or null"));
                }
                Ok(country.to_ascii_uppercase())
            })
            .transpose()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        compare_revision(&transaction, request.expected_revision)?;
        let node = find_node(&transaction, &request.node_id)?;
        if node.country != country {
            transaction.execute(
                "UPDATE nodes SET country = ?1 WHERE id = ?2",
                params![country, request.node_id],
            )?;
            bump(&transaction)?;
            ensure_publishable(&transaction)?;
            access::ensure_publishable(&transaction)?;
        }
        let node = find_node(&transaction, &request.node_id)?;
        transaction.commit()?;
        Ok(node)
    }

    pub fn configuration(&self, token: &str) -> Result<ConfigurationSnapshot, Error> {
        snapshot(&self.connection, &node_id(&self.connection, token)?)
    }

    pub fn acknowledge(
        &mut self,
        token: &str,
        request: &AcknowledgeConfigurationRequest,
    ) -> Result<AcknowledgeConfigurationResponse, Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = node_id(&transaction, token)?;
        let current = snapshot(&transaction, &id)?;
        if current.payload.revision != request.revision || current.digest != request.digest {
            return Err(Error::Conflict);
        }
        transaction.execute(
            "UPDATE nodes SET acknowledged_revision = ?1, acknowledged_digest = ?2 WHERE id = ?3",
            params![request.revision, request.digest, id],
        )?;
        transaction.commit()?;
        Ok(AcknowledgeConfigurationResponse {
            revision: request.revision,
            digest: request.digest.clone(),
        })
    }
}
