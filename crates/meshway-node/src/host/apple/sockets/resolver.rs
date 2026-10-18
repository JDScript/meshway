//! Bounded system bootstrap lookups. libc's resolver cannot be interrupted;
//! cancelled callers discard its result, retain its budget, and Host joins it.
use super::*;
use std::net::ToSocketAddrs;
use tokio_util::sync::CancellationToken;

pub(in crate::host::apple) struct Resolver {
    jobs: Mutex<Jobs>,
    slots: Arc<tokio::sync::Semaphore>,
}
#[derive(Default)]
struct Jobs {
    closed: bool,
    tasks: tokio::task::JoinSet<()>,
}
impl Default for Resolver {
    fn default() -> Self {
        Self {
            jobs: Mutex::new(Jobs::default()),
            slots: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }
}
impl Resolver {
    pub async fn lookup(
        &self,
        stop: CancellationToken,
        host: String,
        port: u16,
        timeout: Duration,
    ) -> io::Result<Vec<SocketAddr>> {
        self.lookup_with(stop, timeout, move || {
            (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.take(16).collect())
        })
        .await
    }
    async fn lookup_with(
        &self,
        stop: CancellationToken,
        timeout: Duration,
        resolve: impl FnOnce() -> io::Result<Vec<SocketAddr>> + Send + 'static,
    ) -> io::Result<Vec<SocketAddr>> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| io::ErrorKind::WouldBlock)?;
        let job_stop = stop.clone();
        let (completed, result) = tokio::sync::oneshot::channel();
        {
            let mut jobs = self.jobs.lock().map_err(|_| io::ErrorKind::Other)?;
            if jobs.closed {
                return Err(io::ErrorKind::NotConnected.into());
            }
            while jobs.tasks.try_join_next().is_some() {}
            jobs.tasks.spawn_blocking(move || {
                let _permit = permit;
                let result = check(&job_stop).and_then(|()| resolve());
                let result = check(&job_stop).and(result);
                let _ = completed.send(result);
            });
        }
        tokio::select! { biased;
            _ = stop.cancelled() => Err(io::ErrorKind::NotConnected.into()),
            result = tokio::time::timeout(timeout, result) => result.map_err(|_| io::ErrorKind::TimedOut)?.map_err(|_| io::ErrorKind::Other)?,
        }
    }
    pub async fn shutdown(&self) {
        self.slots.close();
        let mut tasks = match self.jobs.lock() {
            Ok(mut jobs) => {
                jobs.closed = true;
                std::mem::take(&mut jobs.tasks)
            }
            Err(_) => return,
        };
        while tasks.join_next().await.is_some() {}
    }
}

fn check(stop: &CancellationToken) -> io::Result<()> {
    if stop.is_cancelled() {
        Err(io::ErrorKind::NotConnected.into())
    } else {
        Ok(())
    }
}

pub(in crate::host::apple) struct HttpResolver {
    pub queries: Arc<Resolver>,
    pub stop: CancellationToken,
    pub hosts: BTreeMap<String, Vec<IpAddr>>,
}
impl reqwest::dns::Resolve for HttpResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let queries = self.queries.clone();
        let stop = self.stop.clone();
        let fixed = self.hosts.get(name.as_str()).cloned();
        Box::pin(async move {
            check(&stop)?;
            let addresses = match fixed {
                Some(addresses) => addresses
                    .into_iter()
                    .map(|address| SocketAddr::new(address, 0))
                    .collect(),
                None => {
                    queries
                        .lookup(stop, name.as_str().to_owned(), 0, Duration::from_secs(20))
                        .await?
                }
            };
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Release(Arc<(Mutex<bool>, std::sync::Condvar)>);
    impl Drop for Release {
        fn drop(&mut self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }
    }

    #[tokio::test]
    async fn cancelled_system_lookup_keeps_its_budget_and_is_joined() {
        let resolver = Resolver {
            jobs: Mutex::new(Jobs::default()),
            slots: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        let stop = CancellationToken::new();
        let entered = Arc::new(tokio::sync::Notify::new());
        let ready = entered.clone();
        let release = Release(Arc::new((Mutex::new(false), std::sync::Condvar::new())));
        let gate = release.0.clone();
        let lookup = resolver.lookup_with(stop.clone(), Duration::from_secs(5), move || {
            ready.notify_one();
            let mut open = gate.0.lock().unwrap();
            while !*open {
                open = gate.1.wait(open).unwrap();
            }
            Ok(vec!["127.0.0.1:53".parse().unwrap()])
        });
        tokio::pin!(lookup);
        tokio::select! {
            _ = entered.notified() => {},
            result = &mut lookup => panic!("lookup completed before release: {result:?}"),
        }
        stop.cancel();
        assert_eq!(
            lookup.await.unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
        let next = resolver
            .lookup(
                CancellationToken::new(),
                "localhost".into(),
                53,
                Duration::from_secs(1),
            )
            .await;
        assert_eq!(next.unwrap_err().kind(), io::ErrorKind::WouldBlock);
        let closing = resolver.shutdown();
        tokio::pin!(closing);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(closing.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        drop(release);
        closing.await;
    }
}
