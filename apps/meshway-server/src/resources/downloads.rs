//! Shared bounded, coalesced downloads; cancellation drops the socket future.
use super::download::{Connector, Https};
use super::rulesets::{Prepared, Source};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use meshway_configuration::RuleSet;
use meshway_protocol::resources::ResourceProblem;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Semaphore, watch};

static BUDGET: Semaphore = Semaphore::const_new(4);
/// Conversions and verifications running at once, across every fetch.
const WORKERS: usize = 4;

/// The owner of rule-set CPU work (conversion, artifact verification). That
/// work cannot be cancelled once started, so each job holds its own budget
/// permit until it ends, even after the fetch awaiting it was cancelled or
/// timed out; closing waits for every job.
pub(crate) struct Workers {
    budget: Arc<Semaphore>,
    /// Running jobs; `None` once joined, after which no job starts.
    running: Mutex<Option<tokio::task::JoinSet<()>>>,
}
impl Workers {
    fn new() -> Self {
        Self {
            budget: Arc::new(Semaphore::new(WORKERS)),
            running: Mutex::new(Some(tokio::task::JoinSet::new())),
        }
    }
    /// Run `work` on a blocking thread within the budget; a full budget is
    /// `BudgetExceeded` at once.
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, ResourceProblem> + Send + 'static,
    ) -> Result<T, ResourceProblem> {
        let permit = self
            .budget
            .clone()
            .try_acquire_owned()
            .map_err(|_| ResourceProblem::BudgetExceeded)?;
        let (reply, result) = tokio::sync::oneshot::channel();
        {
            let mut running = self
                .running
                .lock()
                .map_err(|_| ResourceProblem::BudgetExceeded)?;
            let running = running.as_mut().ok_or(ResourceProblem::BudgetExceeded)?;
            while running.try_join_next().is_some() {}
            running.spawn_blocking(move || {
                let _permit = permit;
                let _ = reply.send(work());
            });
        }
        result.await.map_err(|_| ResourceProblem::BudgetExceeded)?
    }
    #[cfg(test)]
    pub(crate) fn available(&self) -> usize {
        self.budget.available_permits()
    }
    async fn join(&self) {
        let running = match self.running.lock() {
            Ok(mut running) => running.take(),
            Err(_) => return,
        };
        if let Some(mut running) = running {
            while running.join_next().await.is_some() {}
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum FetchError {
    Resource(ResourceProblem),
    Cancelled,
}
impl From<ResourceProblem> for FetchError {
    fn from(problem: ResourceProblem) -> Self {
        Self::Resource(problem)
    }
}
type ResultFuture = Shared<BoxFuture<'static, Result<Prepared, FetchError>>>;
struct Flight {
    result: ResultFuture,
}
pub(crate) struct Downloads {
    connector: Arc<dyn Connector>,
    flights: Mutex<BTreeMap<String, Weak<Flight>>>,
    stop: watch::Sender<bool>,
    workers: Arc<Workers>,
}
impl Downloads {
    pub(crate) fn new() -> Arc<Self> {
        // The Server fetches provider catalogs by itself; unit tests never
        // reach the Internet (resource tests inject their own source).
        #[cfg(test)]
        return Self::with_connector(Arc::new(super::download::Offline));
        #[cfg(not(test))]
        Self::with_connector(Arc::new(Https))
    }
    pub(crate) fn with_connector(connector: Arc<dyn Connector>) -> Arc<Self> {
        Arc::new(Self {
            connector,
            flights: Mutex::new(BTreeMap::new()),
            stop: watch::channel(false).0,
            workers: Arc::new(Workers::new()),
        })
    }
    pub(crate) fn close(&self) {
        self.stop.send_replace(true);
    }
    /// After [`Self::close`]: wait for every started conversion.
    pub(crate) async fn join(&self) {
        self.workers.join().await;
    }
    #[cfg(test)]
    pub(crate) fn workers(&self) -> &Arc<Workers> {
        &self.workers
    }
    pub(crate) fn closed(&self) -> bool {
        *self.stop.borrow()
    }
    pub(crate) async fn cancelled(&self) {
        let mut stop = self.stop.subscribe();
        if !*stop.borrow() {
            let _ = stop.changed().await;
        }
    }
    /// A provider's catalog, downloaded and verified within the shared
    /// transfer budget. Only the scheduler fetches catalogs alone, one at a
    /// time, so there is nothing to coalesce.
    pub(crate) async fn catalog(
        &self,
        provider: &meshway_configuration::RuleSetProvider,
    ) -> Result<super::catalogs::VerifiedCatalog, FetchError> {
        let _permit = BUDGET
            .try_acquire()
            .map_err(|_| ResourceProblem::BudgetExceeded)?;
        let mut stop = self.stop.subscribe();
        if *stop.borrow() {
            return Err(FetchError::Cancelled);
        }
        tokio::select! {
            biased;
            _ = stop.changed() => Err(FetchError::Cancelled),
            result = super::catalogs::download_verified(provider, self.connector.as_ref()) => {
                result.map_err(Into::into)
            }
        }
    }
    /// Content of one source ready to bind: converted, or taken from a
    /// verified catalog. Concurrent fetches of one source share a download.
    /// `catalogs` memoizes verified catalogs across the fetches of one
    /// preparation.
    pub(crate) async fn fetch(
        &self,
        source: &Source,
        catalogs: &super::catalogs::Catalogs,
    ) -> Result<Prepared, FetchError> {
        if self.closed() {
            return Err(FetchError::Cancelled);
        }
        if let Source::Inline(set) = source {
            return super::rulesets::inline(set).map_err(Into::into);
        }
        // Updating the interval does not change the source being fetched.
        let key = source.identity();
        let flight = {
            let mut flights = self
                .flights
                .lock()
                .map_err(|_| ResourceProblem::BudgetExceeded)?;
            flights.retain(|_, flight| flight.strong_count() > 0);
            if let Some(flight) = flights.get(&key).and_then(Weak::upgrade) {
                flight
            } else {
                let permit = BUDGET
                    .try_acquire()
                    .map_err(|_| ResourceProblem::BudgetExceeded)?;
                let connector = self.connector.clone();
                let workers = self.workers.clone();
                let catalogs = catalogs.clone();
                let source = source.clone();
                let mut stop = self.stop.subscribe();
                let result = async move {
                    let _permit = permit;
                    if *stop.borrow() {
                        return Err(FetchError::Cancelled);
                    }
                    let work = async {
                        match &source {
                            Source::Remote(RuleSet::Remote { url, format, .. }) => {
                                let bytes = super::download::download(
                                    url,
                                    connector.as_ref(),
                                    meshway_rules::convert::MAX_SOURCE_BYTES,
                                )
                                .await?;
                                let format = format.clone();
                                workers
                                    .run(move || super::rulesets::convert(&bytes, &format))
                                    .await
                            }
                            Source::Provider {
                                name, definition, ..
                            } => {
                                catalogs
                                    .fetch(definition, name, connector.as_ref(), &workers)
                                    .await
                            }
                            Source::Inline(set) | Source::Remote(set) => {
                                super::rulesets::inline(set)
                            }
                        }
                    };
                    tokio::select! {
                        biased;
                        _ = stop.changed() => Err(FetchError::Cancelled),
                        result = work => result.map_err(Into::into),
                    }
                }
                .boxed()
                .shared();
                let flight = Arc::new(Flight { result });
                flights.insert(key, Arc::downgrade(&flight));
                flight
            }
        };
        flight.result.clone().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn a_cancelled_conversion_keeps_its_budget_until_it_ends_and_close_joins_it() {
        let downloads = Downloads::with_connector(Arc::new(Https));
        let workers = downloads.workers().clone();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let (started, running) = tokio::sync::oneshot::channel();
        let job = workers.run(move || {
            let _ = started.send(());
            let _ = gate.recv();
            Ok(())
        });
        // The awaiting fetch gives up (timeout, client gone) mid-conversion.
        let cancelled = async {
            tokio::select! {
                _ = job => panic!("the job cannot finish yet"),
                _ = async { running.await.unwrap(); tokio::time::sleep(Duration::from_millis(50)).await } => {}
            }
        };
        cancelled.await;
        assert_eq!(
            workers.available(),
            WORKERS - 1,
            "the running job keeps its permit"
        );
        // The rest of the budget still bounds new work.
        let mut blockers = Vec::new();
        let mut gates = Vec::new();
        for _ in 1..WORKERS {
            let (release, gate) = std::sync::mpsc::channel::<()>();
            gates.push(release);
            let workers = workers.clone();
            blockers.push(tokio::spawn(async move {
                workers
                    .run(move || {
                        let _ = gate.recv();
                        Ok(())
                    })
                    .await
            }));
        }
        while workers.available() > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(
            workers.run(|| Ok(())).await,
            Err(ResourceProblem::BudgetExceeded)
        );
        downloads.close();
        let joined = tokio::spawn({
            let downloads = downloads.clone();
            async move { downloads.join().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!joined.is_finished(), "close waits for started conversions");
        release.send(()).unwrap();
        for gate in gates {
            gate.send(()).unwrap();
        }
        joined.await.unwrap();
        for blocker in blockers {
            assert_eq!(blocker.await.unwrap(), Ok(()));
        }
        assert_eq!(workers.available(), WORKERS);
        // Joined: nothing starts any more.
        assert_eq!(
            workers.run(|| Ok(())).await,
            Err(ResourceProblem::BudgetExceeded)
        );
    }
}
