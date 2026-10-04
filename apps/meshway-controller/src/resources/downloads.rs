//! Shared bounded, coalesced downloads; cancellation drops the socket future.
use super::download::{Connector, Https};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use meshway_configuration::RuleSet;
use meshway_protocol::resources::{ResourceProblem, RuleSetData};
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
type ResultFuture = Shared<BoxFuture<'static, Result<RuleSetData, FetchError>>>;
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
    pub(crate) async fn fetch(&self, definition: &RuleSet) -> Result<RuleSetData, FetchError> {
        if self.closed() {
            return Err(FetchError::Cancelled);
        }
        let (url, format) = match definition {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
            } => {
                return super::normalize(RuleSetData {
                    ip_cidr: ip_cidr.clone(),
                    domain_suffix: domain_suffix.clone(),
                })
                .map_err(Into::into);
            }
            RuleSet::Remote { url, format, .. } => (url, format),
        };
        // Updating the interval does not change the source/format being fetched.
        let key =
            serde_json::to_string(&(url, format)).map_err(|_| ResourceProblem::InvalidSource)?;
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
                let url = url.clone();
                let format = format.clone();
                let mut stop = self.stop.subscribe();
                let result = async move {
                    let _permit = permit;
                    if *stop.borrow() { return Err(FetchError::Cancelled); }
                    tokio::select! {
                        biased;
                        _ = stop.changed() => Err(FetchError::Cancelled),
                        result = super::download::download(&url, connector.as_ref()) => super::convert(&result?, &format).map_err(Into::into),
                    }
                }.boxed().shared();
                let flight = Arc::new(Flight { result });
                flights.insert(key, Arc::downgrade(&flight));
                flight
            }
        };
        flight.result.clone().await
    }
}
