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
}
impl Downloads {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_connector(Arc::new(Https))
    }
    pub(crate) fn with_connector(connector: Arc<dyn Connector>) -> Arc<Self> {
        Arc::new(Self {
            connector,
            flights: Mutex::new(BTreeMap::new()),
            stop: watch::channel(false).0,
        })
    }
    pub(crate) fn close(&self) {
        self.stop.send_replace(true);
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
                                tokio::task::spawn_blocking({
                                    let format = format.clone();
                                    move || super::rulesets::convert(&bytes, &format)
                                })
                                .await
                                .map_err(|_| ResourceProblem::BudgetExceeded)?
                            }
                            Source::Provider {
                                name, definition, ..
                            } => catalogs.fetch(definition, name, connector.as_ref()).await,
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
