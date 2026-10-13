//! App-owned round-robin network scheduling. One periodic download at a time;
//! all download callers share the global four-transfer budget and coalescer.
use super::{
    downloads::Downloads,
    provider::Provider,
    service::{ResourceProvider, ResourceService, SystemTask},
};
use crate::error::Error;
use crate::store::{self, Handle};
use chrono::Utc;
use meshway_model::NetworkId;
use rusqlite::Transaction;
use std::{sync::Arc, time::Duration};

pub(crate) const MIN_INTERVAL_MS: u64 = 1_000;
const SCAN_PERIOD: Duration = Duration::from_millis(100);
/// When a periodic check is next due (Unix ms): at once (0) without a check,
/// else after the interval (at least `MIN_INTERVAL_MS`) or the failure backoff.
pub(crate) fn due_at(checked: Option<u64>, interval: u64, failures: u32) -> u64 {
    let Some(checked) = checked else {
        return 0;
    };
    let delay = if failures == 0 {
        interval.max(MIN_INTERVAL_MS)
    } else {
        5_000u64
            .saturating_mul(1u64 << failures.saturating_sub(1).min(16))
            .min(300_000)
    };
    checked.saturating_add(delay)
}
/// How often unreferenced artifacts are collected; the first pass runs at start.
const COLLECT_PERIOD: Duration = Duration::from_secs(60 * 60);

/// One unit of periodic work found by a network scan.
enum Task {
    /// A provider catalog effective in the network, fetched on its own.
    Catalog(meshway_configuration::RuleSetProvider),
    RuleSet(SystemTask),
}

pub(crate) struct Scheduler {
    downloads: Arc<Downloads>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Scheduler {
    pub(crate) fn start(handle: Handle, provider: Provider, downloads: Arc<Downloads>) -> Self {
        let service = ResourceService::new(handle.clone(), provider.clone(), downloads.clone());
        let cancellation = downloads.clone();
        let task = tokio::spawn(async move {
            let mut after = None;
            let mut collect_at = tokio::time::Instant::now();
            loop {
                tokio::select! { biased; _ = cancellation.cancelled() => break, _ = tokio::time::sleep(SCAN_PERIOD) => {} }
                if tokio::time::Instant::now() >= collect_at {
                    collect_at += COLLECT_PERIOD;
                    let owner = provider.clone();
                    let collected = tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => break,
                        collected = handle.write(move |tx| {
                            super::artifacts::collect(tx, &owner.sealer, Utc::now().timestamp_millis())
                        }) => collected,
                    };
                    match collected {
                        Ok(0) => {}
                        Ok(count) => {
                            tracing::info!(count, "Unreferenced rule-set artifacts collected")
                        }
                        Err(error) => tracing::warn!(%error, "Rule-set artifact collection failed"),
                    }
                }
                let scanned = provider.clone();
                let scan = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    scan = handle.read(move |tx| scan_network(tx, &scanned, after)) => scan,
                };
                if let Ok((network, task)) = scan {
                    after = network;
                    match task {
                        Some(Task::Catalog(catalog)) => {
                            tokio::select! { biased; _ = cancellation.cancelled() => break, _ = super::catalogs::refresh(&handle, &cancellation, catalog) => {} }
                        }
                        Some(Task::RuleSet(task)) => {
                            tokio::select! { biased; _ = cancellation.cancelled() => break, _ = service.refresh_system(task) => {} }
                        }
                        None => {}
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

enum Next<C, S> {
    Catalog(C),
    Set(S),
}

/// The due task with the earliest deadline, over catalogs and sets alike: a
/// refreshed task's next deadline is after now, so one with a short interval
/// never passes a task that has waited longer. On equal deadlines a catalog
/// goes first (a set of a provider is then prepared against an accepted
/// catalog), then the first in key order.
fn earliest<C, S>(
    catalogs: impl IntoIterator<Item = (u64, C)>,
    sets: impl IntoIterator<Item = (u64, S)>,
    now: u64,
) -> Option<Next<C, S>> {
    let catalog = catalogs
        .into_iter()
        .filter(|(at, _)| *at <= now)
        .min_by_key(|(at, _)| *at);
    let set = sets
        .into_iter()
        .filter(|(at, _)| *at <= now)
        .min_by_key(|(at, _)| *at);
    match (catalog, set) {
        (Some((at, catalog)), Some((set_at, _))) if at <= set_at => Some(Next::Catalog(catalog)),
        (_, Some((_, set))) => Some(Next::Set(set)),
        (catalog, None) => catalog.map(|(_, catalog)| Next::Catalog(catalog)),
    }
}

fn scan_network(
    tx: &Transaction<'_>,
    provider: &Provider,
    after: Option<NetworkId>,
) -> Result<(Option<NetworkId>, Option<Task>), Error> {
    let network = match store::resources::checks::next_network(tx, after)? {
        Some(id) => Some(id),
        None => store::resources::checks::next_network(tx, None)?,
    };
    let Some(network) = network else {
        return Ok((None, None));
    };
    let now = Utc::now();
    // A network with unavailable immutable dependencies must not starve other
    // networks. Its ordinary resource GET continues to report that error.
    let scope = match provider.system_rule_sets(tx, network, now) {
        Ok(scope) => scope,
        Err(_) => return Ok((Some(network), None)),
    };
    let now_ms = now.timestamp_millis().max(0) as u64;
    let catalogs = scope
        .providers
        .values()
        .map(|catalog| Ok((super::catalogs::due_at(tx, catalog)?, catalog)))
        .collect::<Result<Vec<_>, Error>>()?;
    let (created, failures) = store::resources::checks::schedule(tx, network, scope.revision)?;
    let sets = scope.definitions.iter().filter_map(|(key, definition)| {
        let interval = definition.interval_ms()?;
        let view = scope.views.get(key)?;
        // A historical restore may have a valid artifact but no check record
        // for the new version. Schedule its first interval from the source
        // commit, without inventing a fresh download timestamp in State.
        let checked = view
            .checked_at
            .map(|at| at.timestamp_millis().max(0) as u64)
            .or_else(|| view.artifact.as_ref().map(|_| created));
        Some((
            due_at(checked, interval, *failures.get(key).unwrap_or(&0)),
            (key, definition, view),
        ))
    });
    let task = earliest(catalogs, sets, now_ms).map(|next| match next {
        Next::Catalog(catalog) => Task::Catalog(catalog.clone()),
        Next::Set((key, definition, view)) => Task::RuleSet(SystemTask {
            network,
            revision: scope.revision,
            key: key.clone(),
            definition: definition.clone(),
            artifact: view.artifact.as_ref().and_then(|a| a.digest.parse().ok()),
            check_version: *scope.check_versions.get(key).unwrap_or(&0),
        }),
    });
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
    /// Two provider catalogs refreshed every second and a set due after
    /// them: catalogs no longer go first unconditionally, so the set runs as
    /// soon as it is the longest waiting, and every task keeps running.
    #[test]
    fn short_interval_catalogs_do_not_starve_other_tasks() {
        // (interval, last check) of catalogs a, b and set s.
        let mut checked = [("a", 1_000, 0u64), ("b", 1_000, 0), ("s", 60_000, 0)];
        let mut runs = std::collections::BTreeMap::new();
        let mut now = 60_000;
        for _ in 0..200 {
            let at = |i: usize| due_at(Some(checked[i].2), checked[i].1, 0);
            let catalogs = [(at(0), 0), (at(1), 1)];
            let sets = [(at(2), 2)];
            if let Some(next) = earliest(catalogs, sets, now) {
                let i = match next {
                    Next::Catalog(i) | Next::Set(i) => i,
                };
                checked[i].2 = now;
                *runs.entry(checked[i].0).or_insert(0) += 1;
                // A refresh takes longer than the scan period.
                now += 700;
            }
            now += 100;
        }
        assert!(runs["s"] >= 2, "{runs:?}");
        assert!(runs["a"] > 10 && runs["b"] > 10, "{runs:?}");
        // Equal deadlines: the catalog first, then key order.
        assert!(matches!(
            earliest([(5, "c")], [(5, "s")], 5),
            Some(Next::Catalog("c"))
        ));
        assert!(matches!(
            earliest([(6, "c")], [(5, "s")], 9),
            Some(Next::Set("s"))
        ));
        assert!(earliest([(6, "c")], [(7, "s")], 5).is_none());
    }

    #[test]
    fn deadlines_are_clamped_saturating_and_backed_off() {
        let due = |now, checked, interval, failures| now >= due_at(checked, interval, failures);
        assert!(due(0, None, u64::MAX, 0));
        assert!(!due(999, Some(0), 1, 0));
        assert!(due(1000, Some(0), 1, 0));
        assert!(!due(10_000, Some(1), u64::MAX, 0));
        assert!(!due(4999, Some(0), 1, 1));
        assert!(due(5000, Some(0), 1, 1));
        assert!(!due(299_999, Some(0), 1, 16));
        assert!(due(300_000, Some(0), 1, 16));
    }
}
