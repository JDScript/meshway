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
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Backend)
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        unreachable!("this regression substitutes observation I/O only")
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
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

struct FailingOwner {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    observations: Arc<AtomicUsize>,
    recoveries: Arc<AtomicUsize>,
    /// Recovery attempts that fail before one succeeds.
    unavailable: usize,
}

impl Owner for FailingOwner {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        self.observations.fetch_add(1, Ordering::SeqCst);
        Err(SystemDnsError::Backend)
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        let attempt = self.recoveries.fetch_add(1, Ordering::SeqCst);
        if attempt < self.unavailable {
            Err(SystemDnsError::Backend)
        } else {
            Ok(())
        }
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        unreachable!("this regression covers observation and recovery only")
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        unreachable!("this regression covers observation and recovery only")
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn failed_owner_keeps_its_fault_while_it_retries_recovery_until_it_succeeds() {
    let (scopes, _) = watch::channel(None);
    let observations = Arc::new(AtomicUsize::new(0));
    let recoveries = Arc::new(AtomicUsize::new(0));
    let actor = Actor::start(FailingOwner {
        scopes,
        observations: observations.clone(),
        recoveries: recoveries.clone(),
        unavailable: 3,
    });
    let mut fault = actor.fault.clone();
    fault.wait_for(Option::is_some).await.unwrap();
    let mut recovered = actor.recovered.clone();
    tokio::time::timeout(
        RECOVERY_INTERVAL * 5,
        recovered.wait_for(|recovered| *recovered),
    )
    .await
    .expect("an available backend is recovered without a parent retry")
    .unwrap();
    assert_eq!(recoveries.load(Ordering::SeqCst), 4);
    // Recovery is throttled and observes nothing until the parent's hold.
    tokio::time::sleep(RECOVERY_INTERVAL * 3).await;
    assert_eq!(observations.load(Ordering::SeqCst), 1);
    assert_eq!(recoveries.load(Ordering::SeqCst), 4);
    assert_eq!(*fault.borrow(), Some(SystemDnsError::Backend));
    actor.shutdown(true).await.unwrap();
}
