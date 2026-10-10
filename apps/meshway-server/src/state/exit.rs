//! Volatile exit prerequisite projection, fenced by instance and netmap. Only
//! configuration evaluation reads `inputs`; it never compiles a netmap. Exit
//! readiness reaches Nodes as a peer fact (`state::facts`).
use super::{CLOCK_SKEW_SECONDS, REPORT_TTL_SECONDS, ReportAdmission};
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
        payload BLOB NOT NULL, active INTEGER NOT NULL CHECK(active IN(0,1)),
        PRIMARY KEY(network_id,node_id)) STRICT;",
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
        // Preserve its watermark/budget to reject later replay or clock reset.
        return Ok(tx.execute(
            "UPDATE temp.state_exit SET active=0 WHERE network_id=?1 AND node_id=?2 AND active=1",
            params![
                admission.network_id.as_uuid().as_bytes(),
                admission.node_id.as_uuid().as_bytes()
            ],
        )? > 0);
    };
    if capability.instance_id != admission.instance_id
        || !admission.configurations.contains(&capability.netmap)
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
    let Some(reference) = admission.execution else {
        return Err(Error::StateScopeChanged);
    };
    if capability.netmap != reference {
        return Err(Error::StateScopeChanged);
    }
    let netmap = crate::store::netmap::load(
        tx,
        sealer,
        admission.network_id,
        admission.node_id,
        reference,
    )?;
    match &netmap.netmap.exit_role {
        Some(role) if role.generation != capability.role_generation => {
            return Err(Error::StateScopeChanged);
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
            || netmap
                .netmap
                .self_
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
    let previous: Option<(Vec<u8>, i64, bool)> = tx
        .query_row(
            "SELECT payload,expires_at,active FROM temp.state_exit WHERE network_id=?1 AND node_id=?2 AND instance_id=?3 AND generation=?4 AND revision=?5",
            params![
                admission.network_id.as_uuid().as_bytes(),
                admission.node_id.as_uuid().as_bytes(),
                admission.instance_id.as_uuid().as_bytes(),
                admission.instance_generation.get() as i64,
                reference.revision.get() as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let mut expires = (capability.observed_at + Duration::seconds(REPORT_TTL_SECONDS))
        .min(now + Duration::seconds(REPORT_TTL_SECONDS))
        .min(admission.lease_expires_at)
        .timestamp_millis();
    if let Some((old, expiry, active)) = &previous {
        let old: Observation = execution::json(old)?;
        if observation.observed_at < old.observed_at
            || observation.observed_at == old.observed_at && (observation != old || !active)
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
        .is_none_or(|(old, expiry, active)| old != &payload || *expiry != expires || !active);
    tx.execute("INSERT INTO temp.state_exit VALUES(?1,?2,?3,?4,?5,?6,?7,1)
        ON CONFLICT(network_id,node_id) DO UPDATE SET instance_id=excluded.instance_id,generation=excluded.generation,revision=excluded.revision,expires_at=excluded.expires_at,payload=excluded.payload,active=1",
        params![admission.network_id.as_uuid().as_bytes(), admission.node_id.as_uuid().as_bytes(), admission.instance_id.as_uuid().as_bytes(), admission.instance_generation.get() as i64, reference.revision.get() as i64, expires, payload])?;
    Ok(changed)
}

pub(crate) fn inputs(
    tx: &Transaction<'_>,
    network: NetworkId,
    _revision: Revision,
    now: i64,
) -> Result<ExitInputs, Error> {
    initialize(tx)?;
    // A retired watermark is only needed while its original observation could
    // still pass freshness checks, including the allowed source clock skew.
    tx.execute(
        "DELETE FROM temp.state_exit WHERE expires_at<=?1",
        [now.saturating_sub((REPORT_TTL_SECONDS + CLOCK_SKEW_SECONDS) * 1000)],
    )?;
    let mut inputs = ExitInputs::default();
    let mut statement = tx.prepare("SELECT node_id,instance_id,instance_expires_at FROM node_execution WHERE network_id=?1 AND instance_expires_at>?2 AND instance_id IS NOT NULL")?;
    let mut rows = statement.query(params![network.as_uuid().as_bytes(), now])?;
    while let Some(row) = rows.next()? {
        let node = execution::id(row.get(0)?)?;
        inputs.instances.insert(node, execution::id(row.get(1)?)?);
    }
    drop(rows);
    let mut statement = tx.prepare("SELECT s.node_id,s.payload,min(s.expires_at,e.instance_expires_at) FROM temp.state_exit s JOIN node_execution e ON e.network_id=s.network_id AND e.node_id=s.node_id AND e.instance_id=s.instance_id AND e.generation=s.generation JOIN node_netmaps n ON n.network_id=s.network_id AND n.node_id=s.node_id AND n.revision=s.revision WHERE s.network_id=?1 AND s.active=1 AND s.expires_at>?2 AND e.instance_expires_at>?2 AND n.withdrawn_at IS NULL AND n.serial=(SELECT max(h.serial) FROM node_netmaps h WHERE h.network_id=n.network_id AND h.node_id=n.node_id)")?;
    let mut rows = statement.query(params![network.as_uuid().as_bytes(), now])?;
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
