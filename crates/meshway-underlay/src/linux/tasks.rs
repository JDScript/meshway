use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Bounded and joinable, including producers whose original waiter disappeared.
#[derive(Clone)]
pub(super) struct Tasks {
    inner: Arc<Mutex<JoinSet<()>>>,
    pub stop: CancellationToken,
    limit: usize,
}

impl Tasks {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(JoinSet::new())),
            stop: CancellationToken::new(),
            limit,
        }
    }

    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> Result<()> {
        let mut tasks = self.inner.lock().map_err(|_| Error::Worker)?;
        while let Some(result) = tasks.try_join_next() {
            if result.is_err() {
                self.stop.cancel();
            }
        }
        if self.stop.is_cancelled() {
            return Err(Error::Worker);
        }
        if tasks.len() >= self.limit {
            return Err(Error::Capacity);
        }
        let stop = self.stop.clone();
        tasks.spawn(async move {
            tokio::select! { biased;
                _ = stop.cancelled() => {},
                () = future => {},
            }
        });
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        let mut tasks = {
            let mut guard = self.inner.lock().map_err(|_| Error::Worker)?;
            std::mem::take(&mut *guard)
        };
        let mut failed = false;
        while let Some(result) = tasks.join_next().await {
            failed |= result.is_err();
        }
        if failed { Err(Error::Worker) } else { Ok(()) }
    }
}

impl hickory_net::runtime::Spawn for Tasks {
    fn spawn_bg(&mut self, future: impl Future<Output = ()> + Send + 'static) {
        // Dropping a rejected DNS producer closes its exchange; no detached task.
        if self.spawn(future).is_err() {
            self.stop.cancel();
        }
    }
}
