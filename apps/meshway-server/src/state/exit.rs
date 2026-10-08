//! Volatile exit prerequisite projection, fenced by instance and role revision.
use super::{REPORT_TTL_SECONDS, ReportAdmission};
use crate::{error::Error, secrets::Sealer, store::execution};
use chrono::{DateTime, Duration, Utc};
use meshway_model::{
    NetworkId, NodeId, Revision,
    exit::{EXIT_IPV4_FEATURE, EXIT_IPV4_VERSION, ExitFamily, ExitStatus},
};
use meshway_policy::compile::ExitInputs;
use meshway_protocol::state::StateReport;
use rusqlite::{OptionalExtension, Transaction, params};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct Observation {
    status: ExitStatus,
    observed_at: DateTime<Utc>,
}

fn initialize(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS state_exit (
        network_id BLOB NOT NULL, node_id BLOB NOT NULL, instance_id BLOB NOT NULL,
        generation INTEGER NOT NULL, revision INTEGER NOT NULL, expires_at INTEGER NOT NULL,
        payload BLOB NOT NULL, PRIMARY KEY(network_id,node_id)) STRICT;",
    )?;
    Ok(())
}

pub(super) fn replace(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    admission: &ReportAdmission,
    report: &StateReport,
    now: DateTime<Utc>,
) -> Result<bool, Error> {
    initialize(tx)?;
    let Some(capability) = &report.state.exit_capability else {
        // Reports are complete snapshots: unknown must withdraw a previous
        // readiness observation, rather than silently reusing it until expiry.
        return Ok(tx.execute(
            "DELETE FROM temp.state_exit WHERE network_id=?1 AND node_id=?2",
            params![
                admission.network_id.as_uuid().as_bytes(),
                admission.node_id.as_uuid().as_bytes()
            ],
        )? > 0);
    };
    if capability.instance_id != admission.instance_id
        || !admission.configurations.contains(&capability.target)
        || capability.observed_at > report.observed_at
        || capability.observed_at + Duration::seconds(REPORT_TTL_SECONDS) <= now
        || capability.supported_families.len() > 2
        || capability.installed_families.len() > 2
        || capability
            .supported_families
            .iter()
            .enumerate()
            .any(|(i, family)| capability.supported_families[..i].contains(family))
        || capability
            .installed_families
            .iter()
            .enumerate()
            .any(|(i, family)| {
                capability.installed_families[..i].contains(family)
                    || !capability.supported_families.contains(family)
            })
    {
        return Err(Error::InvalidRequest);
    }
    let Some((_, target)) =
        execution::target_for_node(tx, sealer, admission.network_id, admission.node_id)?
    else {
        return Err(Error::StateScopeChanged);
    };
    let reference = meshway_model::ConfigurationRef {
        revision: target.configuration.revision,
        execution_publication: target.configuration.publication,
        digest: target.digest,
    };
    if capability.target != reference {
        return Err(Error::StateScopeChanged);
    }
    match &target.configuration.exit_role {
        Some(role) if role.instance_id != capability.instance_id => {
            return Err(Error::InvalidSession);
        }
        Some(role)
            if capability
                .installed_families
                .iter()
                .any(|family| family != &role.family) =>
        {
            return Err(Error::InvalidRequest);
        }
        Some(_) => {}
        None if capability.status == ExitStatus::NotRequested
            && capability.installed_families.is_empty() => {}
        None => return Err(Error::InvalidRequest),
    }
    if !capability.installed_families.is_empty()
        && report.state.configuration.applied.as_ref() != Some(&reference)
    {
        return Err(Error::InvalidRequest);
    }
    if capability.status == ExitStatus::Ready
        && (!capability.supported_families.contains(&ExitFamily::Ipv4)
            || target
                .configuration
                .self_keys
                .capabilities
                .get(EXIT_IPV4_FEATURE)
                != Some(&meshway_model::state::Capability::Feature {
                    version: EXIT_IPV4_VERSION,
                }))
    {
        return Err(Error::InvalidRequest);
    }
    let observation = Observation {
        status: capability.status.clone(),
        observed_at: capability.observed_at,
    };
    let payload = execution::bytes(&observation)?;
    let previous: Option<(Vec<u8>, i64)> = tx
        .query_row(
            "SELECT payload,expires_at FROM temp.state_exit WHERE network_id=?1 AND node_id=?2 AND instance_id=?3 AND generation=?4 AND revision=?5",
            params![
                admission.network_id.as_uuid().as_bytes(),
                admission.node_id.as_uuid().as_bytes(),
                admission.instance_id.as_uuid().as_bytes(),
                admission.instance_generation.get() as i64,
                reference.revision.get() as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let mut expires = (capability.observed_at + Duration::seconds(REPORT_TTL_SECONDS))
        .min(now + Duration::seconds(REPORT_TTL_SECONDS))
        .min(admission.lease_expires_at)
        .timestamp_millis();
    if let Some((old, expiry)) = &previous {
        let old: Observation = execution::json(old)?;
        if observation.observed_at < old.observed_at
            || observation.observed_at == old.observed_at && observation != old
        {
            return Err(Error::InvalidRequest);
        }
        if observation == old {
            expires = expires.min(*expiry);
        }
    }
    // Refreshing a real observation extends the prerequisite budget in permits.
    // A replay preserves observed_at and therefore cannot manufacture freshness.
    let changed = previous
        .as_ref()
        .is_none_or(|(old, expiry)| old != &payload || *expiry != expires);
    tx.execute("INSERT INTO temp.state_exit VALUES(?1,?2,?3,?4,?5,?6,?7)
        ON CONFLICT(network_id,node_id) DO UPDATE SET instance_id=excluded.instance_id,generation=excluded.generation,revision=excluded.revision,expires_at=excluded.expires_at,payload=excluded.payload",
        params![admission.network_id.as_uuid().as_bytes(), admission.node_id.as_uuid().as_bytes(), admission.instance_id.as_uuid().as_bytes(), admission.instance_generation.get() as i64, reference.revision.get() as i64, expires, payload])?;
    Ok(changed)
}

pub(crate) fn inputs(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    now: i64,
) -> Result<ExitInputs, Error> {
    initialize(tx)?;
    let mut inputs = ExitInputs::default();
    let mut statement = tx.prepare("SELECT node_id,instance_id FROM node_execution WHERE network_id=?1 AND instance_expires_at>?2 AND instance_id IS NOT NULL")?;
    let mut rows = statement.query(params![network.as_uuid().as_bytes(), now])?;
    while let Some(row) = rows.next()? {
        inputs
            .instances
            .insert(execution::id(row.get(0)?)?, execution::id(row.get(1)?)?);
    }
    drop(rows);
    let mut statement =
        tx.prepare("SELECT node_id,version FROM routing_selections WHERE network_id=?1")?;
    let mut rows = statement.query(params![network.as_uuid().as_bytes()])?;
    while let Some(row) = rows.next()? {
        inputs
            .selection_generations
            .insert(execution::id(row.get(0)?)?, row.get::<_, i64>(1)? as u64);
    }
    drop(rows);
    let mut statement = tx.prepare("SELECT s.node_id,s.payload,min(s.expires_at,e.instance_expires_at) FROM temp.state_exit s JOIN node_execution e ON e.network_id=s.network_id AND e.node_id=s.node_id AND e.instance_id=s.instance_id AND e.generation=s.generation WHERE s.network_id=?1 AND s.revision=?2 AND s.expires_at>?3 AND e.instance_expires_at>?3")?;
    let mut rows = statement.query(params![
        network.as_uuid().as_bytes(),
        revision.get() as i64,
        now
    ])?;
    while let Some(row) = rows.next()? {
        let node: NodeId = execution::id(row.get(0)?)?;
        match execution::json::<Observation>(&row.get::<_, Vec<u8>>(1)?)?.status {
            ExitStatus::Ready => {
                inputs
                    .ready_until_ms
                    .insert(node, row.get::<_, i64>(2)? as u64);
            }
            ExitStatus::Unavailable { reason } => {
                inputs.unavailable_reasons.insert(node, reason);
            }
            _ => {}
        }
    }
    Ok(inputs)
}
