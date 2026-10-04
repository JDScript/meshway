//! App-owned round-robin network scheduling. One periodic download at a time;
//! all download callers share the global four-transfer budget and coalescer.
use super::{
    downloads::Downloads,
    provider::Provider,
    service::{ResourceProvider, ResourceService, SystemTask},
};
use crate::error::Error;
use crate::{state::service::Runtime, store::Handle};
use chrono::Utc;
use meshway_configuration::RuleSet;
use meshway_model::NetworkId;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

pub(crate) const MIN_INTERVAL_MS: u64 = 1_000;
const SCAN_PERIOD: Duration = Duration::from_millis(100);
pub(crate) fn due(now: u64, checked: Option<u64>, interval: u64, failures: u32) -> bool {
    let Some(checked) = checked else {
        return true;
    };
    let delay = if failures == 0 {
        interval.max(MIN_INTERVAL_MS)
    } else {
        5_000u64
            .saturating_mul(1u64 << failures.saturating_sub(1).min(16))
            .min(300_000)
    };
    now >= checked.saturating_add(delay)
}
pub(crate) struct Scheduler {
    downloads: Arc<Downloads>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Scheduler {
    pub(crate) fn start(
        handle: Handle,
        provider: Provider,
        runtime: Arc<Mutex<Runtime>>,
        downloads: Arc<Downloads>,
    ) -> Self {
        let service =
            ResourceService::new(handle.clone(), provider.clone(), runtime, downloads.clone());
        let cancellation = downloads.clone();
        let task = tokio::spawn(async move {
            let mut after = None;
            loop {
                tokio::select! { biased; _ = cancellation.cancelled() => break, _ = tokio::time::sleep(SCAN_PERIOD) => {} }
                let provider = provider.clone();
                let scan = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    scan = handle.execute(move |db| scan_network(db, &provider, after)) => scan,
                };
                if let Ok(Ok((network, task))) = scan {
                    after = network;
                    if let Some(task) = task {
                        tokio::select! { biased; _ = cancellation.cancelled() => break, _ = service.refresh_system(task) => {} }
                    }
                }
            }
        });
        Self {
            downloads,
            task: Some(task),
        }
    }
    pub(crate) async fn shutdown(mut self) {
        self.downloads.close();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

fn scan_network(
    db: &mut Connection,
    provider: &Provider,
    after: Option<NetworkId>,
) -> Result<(Option<NetworkId>, Option<SystemTask>), Error> {
    let tx = db.transaction()?;
    let next = |after: Option<NetworkId>| -> Result<Option<NetworkId>, Error> {
        let bytes: Option<Vec<u8>> = tx.query_row(
            "SELECT id FROM networks WHERE deleted_at IS NULL AND (?1 IS NULL OR id>?1) ORDER BY id LIMIT 1",
            [after.map(|id| id.as_uuid().as_bytes().to_vec())],
            |row| row.get(0),
        ).optional()?;
        bytes.map(crate::store::execution::id).transpose()
    };
    let network = match next(after)? {
        Some(id) => Some(id),
        None => next(None)?,
    };
    let Some(network) = network else {
        return Ok((None, None));
    };
    let now = Utc::now();
    // A network with unavailable immutable dependencies must not starve other
    // networks. Its ordinary resource GET continues to report that error.
    let scope = match provider.system_rule_sets(&tx, network, now) {
        Ok(scope) => scope,
        Err(_) => return Ok((Some(network), None)),
    };
    let created: i64 = tx.query_row(
        "SELECT created_at FROM configuration_versions WHERE network_id=?1 AND revision=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            scope.revision.get() as i64
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
                scope.revision.get() as i64
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )?
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let task = scope
        .definitions
        .iter()
        .filter_map(|(key, definition)| {
            let RuleSet::Remote {
                update_interval, ..
            } = definition
            else {
                return None;
            };
            let view = scope.views.get(key)?;
            // A historical restore may have a valid artifact but no check record
            // for the new version. Schedule its first interval from the source
            // commit, without inventing a fresh download timestamp in State.
            let checked = view
                .checked_at
                .map(|at| at.timestamp_millis().max(0) as u64)
                .or_else(|| view.artifact.as_ref().map(|_| created));
            due(
                now.timestamp_millis().max(0) as u64,
                checked,
                update_interval.as_millis(),
                *failures.get(key).unwrap_or(&0),
            )
            .then(|| SystemTask {
                network,
                revision: scope.revision,
                key: key.clone(),
                definition: definition.clone(),
                artifact: view.artifact.as_ref().map(|a| a.id),
                checked_at: view.checked_at,
                check_version: *scope.check_versions.get(key).unwrap_or(&0),
            })
        })
        .min_by_key(|task| (task.checked_at, task.key.clone()));
    Ok((Some(network), task))
}
impl Drop for Scheduler {
    fn drop(&mut self) {
        self.downloads.close();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadlines_are_clamped_saturating_and_backed_off() {
        assert!(!due(999, Some(0), 1, 0));
        assert!(due(1000, Some(0), 1, 0));
        assert!(!due(10_000, Some(1), u64::MAX, 0));
        assert!(!due(4999, Some(0), 1, 1));
        assert!(due(5000, Some(0), 1, 1));
        assert!(!due(299_999, Some(0), 1, 16));
        assert!(due(300_000, Some(0), 1, 16));
    }
}
