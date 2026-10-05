use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SlowOwner {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    entered: mpsc::Sender<()>,
    observations: Arc<AtomicUsize>,
    joined: oneshot::Sender<bool>,
    stall: bool,
}

impl Owner for SlowOwner {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        self.entered.send(()).await.unwrap();
        if self.stall {
            std::future::pending().await
        } else {
            // Every observation exceeds the interval, including the second
            // one if the actor incorrectly admits it ahead of queued shutdown.
            tokio::time::sleep(super::super::OBSERVE_INTERVAL * 2).await;
            Ok(())
        }
    }
    async fn hold(
        &mut self,
        _: &NodeConfiguration,
    ) -> Result<Vec<meshway_dns::ResolverDemand>, SystemDnsError> {
        unreachable!("this regression substitutes observation I/O only")
    }
    async fn install(
        &mut self,
        _: ExecutionSnapshot,
        _: &BTreeMap<ResolverSource, RequestId>,
    ) -> Result<(), SystemDnsError> {
        unreachable!("this regression substitutes observation I/O only")
    }
    async fn shutdown(self, keep: bool) -> Result<(), SystemDnsError> {
        self.joined.send(keep).unwrap();
        Ok(())
    }
}

#[tokio::test]
async fn actual_actor_serves_shutdown_after_one_slow_observation_and_joins() {
    shutdown_during_observation(false).await;
}

#[tokio::test]
async fn actual_actor_fences_a_stalled_observation_then_serves_shutdown() {
    shutdown_during_observation(true).await;
}

async fn shutdown_during_observation(stall: bool) {
    let (scopes, _) = watch::channel(None);
    let (entered, mut observing) = mpsc::channel(8);
    let (joined, join) = oneshot::channel();
    let observations = Arc::new(AtomicUsize::new(0));
    let actor = Actor::start(SlowOwner {
        scopes,
        entered,
        joined,
        observations: observations.clone(),
        stall,
    });
    let fault = actor.fault.clone();
    tokio::time::timeout(Duration::from_secs(3), observing.recv())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(
        OBSERVE_TIMEOUT + Duration::from_secs(2),
        actor.shutdown(false),
    )
    .await
    .expect("overdue observations must not starve the command queue")
    .unwrap();
    assert!(!join.await.unwrap());
    assert_eq!(observations.load(Ordering::SeqCst), 1);
    assert_eq!(fault.borrow().is_some(), stall);
}
