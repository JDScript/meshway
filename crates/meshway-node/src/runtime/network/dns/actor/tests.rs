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
    let actor_detail = actor.detail.clone();
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
    if stall {
        // A stalled observation names the deadline it missed.
        let detail = actor_detail.borrow().clone().unwrap();
        assert_eq!(detail.check, "observe_deadline");
        assert_eq!(detail.error, SystemDnsError::Backend);
    }
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
    fn failure(&self) -> Option<SystemDnsFailure> {
        Some(
            SystemDnsFailure::new(SystemDnsError::Backend, "anchor_netlink")
                .actual("dump failed: kernel netlink operation failed")
                .source("rtnetlink RTM_GETROUTE"),
        )
    }
}

#[tokio::test]
async fn failed_owner_keeps_its_fault_while_it_retries_recovery_until_it_succeeds() {
    let (scopes, _) = watch::channel(None);
    let observations = Arc::new(AtomicUsize::new(0));
    let recoveries = Arc::new(AtomicUsize::new(0));
    let mut actor = Actor::start(FailingOwner {
        scopes,
        observations: observations.clone(),
        recoveries: recoveries.clone(),
        unavailable: 1,
    });
    let mut fault = actor.fault.clone();
    fault.wait_for(Option::is_some).await.unwrap();
    tokio::time::timeout(
        RECOVERY_INTERVAL / 2,
        super::Health::new(actor.fault.clone(), actor.recovered.clone()).failed(),
    )
    .await
    .expect("a fault not yet re-proved is a failure");
    // The fault carries the check the owner recorded, for the report.
    let detail = actor.detail.borrow().clone().unwrap();
    assert_eq!(detail.check, "anchor_netlink");
    assert_eq!(detail.source.as_deref(), Some("rtnetlink RTM_GETROUTE"));
    let mut recovered = actor.recovered.clone();
    tokio::time::timeout(
        RECOVERY_INTERVAL * 3,
        recovered.wait_for(|recovered| *recovered),
    )
    .await
    .expect("an available backend is recovered without a parent retry")
    .unwrap();
    assert_eq!(recoveries.load(Ordering::SeqCst), 2);
    // Recovery is throttled and observes nothing until the parent's hold.
    tokio::time::sleep(RECOVERY_INTERVAL * 2).await;
    assert_eq!(observations.load(Ordering::SeqCst), 1);
    assert_eq!(recoveries.load(Ordering::SeqCst), 2);
    assert_eq!(*fault.borrow(), Some(SystemDnsError::Backend));
    // Not yet acknowledged, the re-proved fault is still the plane's failure.
    tokio::time::timeout(
        RECOVERY_INTERVAL,
        super::Health::new(actor.fault.clone(), actor.recovered.clone()).failed(),
    )
    .await
    .expect("an unacknowledged re-proved fault is a failure");
    // The plane sees that fault once (`Attachment::acknowledge`): re-proved,
    // it is a recovery and no longer a failure while it waits for the hold.
    actor.fault.borrow_and_update();
    let health = super::Health::new(actor.fault.clone(), actor.recovered.clone());
    assert!(
        tokio::time::timeout(RECOVERY_INTERVAL, health.clone().failed())
            .await
            .is_err(),
        "a re-proved fault is not a failure"
    );
    tokio::time::timeout(RECOVERY_INTERVAL, health.recovered())
        .await
        .expect("the recovery stays until the hold");
    actor.shutdown(true).await.unwrap();
}

/// An owner whose hold and installation fail on a recorded check.
struct RefusingOwner {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    install: bool,
}

impl Owner for RefusingOwner {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Backend)
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        if self.install {
            Ok(())
        } else {
            Err(SystemDnsError::Backend)
        }
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
    fn failure(&self) -> Option<SystemDnsFailure> {
        Some(
            SystemDnsFailure::new(SystemDnsError::Backend, "link_settings")
                .source("resolve1 SetLinkDNS"),
        )
    }
}

fn compiled() -> meshway_configuration::netmap::CompiledNetmap {
    use meshway_test_server::configuration::{compile_one, network, node, tag_node, trusted};
    let mut cfg = network("dns-actor.internal");
    cfg.nodes.insert(
        node(1),
        tag_node("a", 1, std::collections::BTreeMap::new(), false),
    );
    let inventory = [(node(1), trusted(1, false))].into_iter().collect();
    compile_one(
        "00000000-0000-4003-8000-000000000009".parse().unwrap(),
        &cfg,
        &inventory,
        node(1),
        None,
    )
}

/// F6a B9: a failed hold or installation publishes its recorded check
/// before the plane sees the answer (`lastErrorDetail` is never empty over
/// a known reason); a later successful installation clears it.
#[tokio::test]
async fn a_failed_hold_or_installation_publishes_its_check_before_answering() {
    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(RefusingOwner {
        scopes,
        install: false,
    });
    let compiled = std::sync::Arc::new(compiled());
    let netmap = compiled.netmap.as_ref().clone();
    assert_eq!(actor.hold(&netmap).await, Err(SystemDnsError::Backend));
    let detail = actor.detail.borrow().clone().expect("the hold's check");
    assert_eq!(detail.check, "link_settings");
    assert_eq!(detail.source.as_deref(), Some("resolve1 SetLinkDNS"));
    let desired = Desired::new(&compiled, &Default::default()).unwrap();
    assert_eq!(
        actor.install(desired.clone()).await,
        Err(SystemDnsError::Backend)
    );
    assert_eq!(
        actor.detail.borrow().as_ref().map(|failure| failure.check),
        Some("link_settings")
    );
    actor.shutdown(true).await.unwrap();

    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(RefusingOwner {
        scopes,
        install: true,
    });
    assert!(actor.hold(&netmap).await.is_err());
    assert!(actor.detail.borrow().is_some());
    actor.install(desired).await.unwrap();
    assert!(actor.detail.borrow().is_none(), "success clears it");
    actor.shutdown(true).await.unwrap();
}
