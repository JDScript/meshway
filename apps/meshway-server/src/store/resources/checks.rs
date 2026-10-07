use crate::error::Error;
use chrono::{DateTime, Utc};
use meshway_model::{NetworkId, RequestId, Revision, UserId};
use meshway_protocol::resources::ResourceProblem;
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;

pub(crate) struct Check {
    pub(crate) problem: Option<ResourceProblem>,
    pub(crate) checked_at: DateTime<Utc>,
    pub(crate) last_success_at: Option<DateTime<Utc>>,
    pub(crate) version: i64,
}

pub(crate) fn inherit(
    tx: &Transaction<'_>,
    network: NetworkId,
    previous: Revision,
    revision: Revision,
    key: &str,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,problem,checked_at,last_success_at,failures,check_version)
                 SELECT network_id,?1,rule_set_key,problem,checked_at,last_success_at,failures,check_version FROM rule_set_checks
                 WHERE network_id=?2 AND revision=?3 AND rule_set_key=?4",
        params![
            revision.get() as i64,
            network.as_uuid().as_bytes().as_slice(),
            previous.get() as i64,
            key,
        ],
    )?;
    Ok(())
}

pub(crate) fn load(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    key: &str,
) -> Result<Option<Check>, Error> {
    let check: Option<(Option<String>, i64, Option<i64>, i64)> = tx.query_row(
        "SELECT problem,checked_at,last_success_at,check_version FROM rule_set_checks WHERE network_id=?1 AND revision=?2 AND rule_set_key=?3",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64, key],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional()?;
    check
        .map(|(problem, checked_at, last_success_at, version)| {
            Ok(Check {
                problem: problem
                    .map(|value| {
                        serde_json::from_str(&value)
                            .map_err(|_| Error::decode("rule_set_checks.problem"))
                    })
                    .transpose()?,
                checked_at: DateTime::from_timestamp_millis(checked_at)
                    .ok_or(Error::decode("rule_set_checks.checked_at"))?,
                last_success_at: last_success_at
                    .map(|at| {
                        DateTime::from_timestamp_millis(at)
                            .ok_or(Error::decode("rule_set_checks.last_success_at"))
                    })
                    .transpose()?,
                version,
            })
        })
        .transpose()
}

pub(crate) fn record(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
    key: &str,
    problem: Option<ResourceProblem>,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    let stored_problem = problem
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|_| Error::encode("rule set check problem"))?;
    tx.execute(
        "INSERT INTO rule_set_checks(network_id,revision,rule_set_key,problem,checked_at,last_success_at,failures)
                     VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(network_id,revision,rule_set_key)
                     DO UPDATE SET problem=excluded.problem,checked_at=excluded.checked_at,
                        last_success_at=COALESCE(excluded.last_success_at,rule_set_checks.last_success_at),
                        failures=CASE WHEN excluded.problem IS NULL THEN 0 ELSE min(rule_set_checks.failures+1,16) END,
                        check_version=rule_set_checks.check_version+1",
        params![
            network.as_uuid().as_bytes().as_slice(),
            revision.get() as i64,
            key,
            stored_problem,
            now.timestamp_millis(),
            problem.is_none().then_some(now.timestamp_millis()),
            i64::from(problem.is_some()),
        ],
    )?;
    Ok(())
}

pub(crate) fn next_network(
    tx: &Transaction<'_>,
    after: Option<NetworkId>,
) -> Result<Option<NetworkId>, Error> {
    let bytes: Option<Vec<u8>> = tx.query_row(
        "SELECT id FROM networks WHERE deleted_at IS NULL AND (?1 IS NULL OR id>?1) ORDER BY id LIMIT 1",
        [after.map(|id| id.as_uuid().as_bytes().to_vec())],
        |row| row.get(0),
    ).optional()?;
    bytes.map(crate::store::execution::id).transpose()
}

pub(crate) fn schedule(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<(u64, BTreeMap<String, u32>), Error> {
    let created: i64 = tx.query_row(
        "SELECT created_at FROM configuration_versions WHERE network_id=?1 AND revision=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            revision.get() as i64
        ],
        |row| row.get(0),
    )?;
    let created =
        u64::try_from(created).map_err(|_| Error::decode("dataset_artifacts.created_at"))?;
    let mut statement = tx.prepare(
        "SELECT rule_set_key,failures FROM rule_set_checks WHERE network_id=?1 AND revision=?2",
    )?;
    let failures = statement
        .query_map(
            params![
                network.as_uuid().as_bytes().as_slice(),
                revision.get() as i64
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )?
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok((created, failures))
}

/// How long a manual refresh request ID stays recognized.
const REFRESH_REQUEST_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;

/// The key a user's earlier refresh request named, if the request ID is
/// still remembered.
pub(crate) fn refresh_request(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    request: RequestId,
    now: DateTime<Utc>,
) -> Result<Option<String>, Error> {
    tx.query_row(
        "SELECT rule_set_key FROM rule_set_refreshes WHERE network_id=?1 AND user_id=?2 AND request_id=?3 AND created_at>?4",
        params![
            network.as_uuid().as_bytes().as_slice(),
            user.as_uuid().as_bytes().as_slice(),
            request.as_uuid().as_bytes().as_slice(),
            now.timestamp_millis() - REFRESH_REQUEST_RETENTION_MS,
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

/// Remembers a performed refresh. Returns false when the same request ID was
/// recorded meanwhile, so the caller treats its own work as the replay.
pub(crate) fn record_refresh_request(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    request: RequestId,
    key: &str,
    now: DateTime<Utc>,
) -> Result<bool, Error> {
    tx.execute(
        "DELETE FROM rule_set_refreshes WHERE created_at<=?1",
        [now.timestamp_millis() - REFRESH_REQUEST_RETENTION_MS],
    )?;
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO rule_set_refreshes(network_id,user_id,request_id,rule_set_key,created_at) VALUES(?1,?2,?3,?4,?5)",
        params![
            network.as_uuid().as_bytes().as_slice(),
            user.as_uuid().as_bytes().as_slice(),
            request.as_uuid().as_bytes().as_slice(),
            key,
            now.timestamp_millis(),
        ],
    )?;
    Ok(inserted == 1)
}
