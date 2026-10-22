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
    let state = actor.state.clone();
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
    assert_eq!(state.borrow().fault.is_some(), stall);
    if stall {
        // A stalled observation names the deadline it missed.
        let detail = state.borrow().detail.clone().unwrap();
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
    let actor = Actor::start(FailingOwner {
        scopes,
        observations: observations.clone(),
        recoveries: recoveries.clone(),
        unavailable: 1,
    });
    let mut state = actor.state.clone();
    state.wait_for(|fact| fact.fault.is_some()).await.unwrap();
    assert!(
        !state.borrow().health.is_healthy(),
        "a fault not yet re-proved is a failure"
    );
    let epoch = state.borrow().epoch;
    assert_eq!(epoch, 1, "one fault");
    // The fault carries the check the owner recorded, for the report.
    let detail = state.borrow().detail.clone().unwrap();
    assert_eq!(detail.check, "anchor_netlink");
    assert_eq!(detail.source.as_deref(), Some("rtnetlink RTM_GETROUTE"));
    assert!(matches!(
        state.borrow().health,
        meshway_model::recovery::Health::Retrying(_)
    ));
    tokio::time::timeout(
        RECOVERY_INTERVAL * 3,
        state.wait_for(|fact| fact.health.is_healthy()),
    )
    .await
    .expect("an available backend is recovered without a parent retry")
    .unwrap();
    assert_eq!(recoveries.load(Ordering::SeqCst), 2);
    // Recovery is throttled and observes nothing until the parent's hold.
    tokio::time::sleep(RECOVERY_INTERVAL * 2).await;
    assert_eq!(observations.load(Ordering::SeqCst), 1);
    assert_eq!(recoveries.load(Ordering::SeqCst), 2);
    // Re-proved, the fault stays (one epoch, healthy) until the plane's hold:
    // a failure once for a plane that has not acknowledged it, then one
    // recovery (worker::lifecycle).
    assert_eq!(state.borrow().fault, Some(SystemDnsError::Backend));
    assert_eq!(state.borrow().epoch, epoch, "the same fault");
    assert!(state.borrow().health.is_healthy());
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
/// a known reason). RC-1 M6 (G2): a failure of the system is the owner's
/// fault, which it re-proves itself (a recovery edge for the plane); a later
/// successful hold and installation clear it.
#[tokio::test]
async fn a_failed_hold_or_installation_publishes_its_check_and_is_recovered() {
    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(RefusingOwner {
        scopes,
        install: false,
    });
    let compiled = std::sync::Arc::new(compiled());
    let netmap = compiled.netmap.as_ref().clone();
    assert_eq!(actor.hold(&netmap).await, Err(SystemDnsError::Backend));
    let detail = actor
        .state
        .borrow()
        .detail
        .clone()
        .expect("the hold's check");
    assert_eq!(detail.check, "link_settings");
    assert_eq!(detail.source.as_deref(), Some("resolve1 SetLinkDNS"));
    assert_eq!(actor.state.borrow().fault, Some(SystemDnsError::Backend));
    let mut state = actor.state.clone();
    tokio::time::timeout(
        RECOVERY_INTERVAL * 3,
        state.wait_for(|fact| fact.health.is_healthy()),
    )
    .await
    .expect("a failed hold has an owner that recovers it")
    .unwrap();
    actor.shutdown(true).await.unwrap();

    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(RefusingOwner {
        scopes,
        install: true,
    });
    let desired = Desired::new(&compiled, &Default::default()).unwrap();
    assert!(actor.hold(&netmap).await.is_err());
    assert!(actor.state.borrow().detail.is_some());
    let mut state = actor.state.clone();
    tokio::time::timeout(
        RECOVERY_INTERVAL * 3,
        state.wait_for(|fact| fact.health.is_healthy()),
    )
    .await
    .unwrap()
    .unwrap();
    actor.install(desired).await.unwrap();
    assert!(
        actor.state.borrow().fault.is_none() && actor.state.borrow().detail.is_none(),
        "a successful installation supersedes the fault"
    );
    actor.shutdown(true).await.unwrap();
}

/// An owner whose hold succeeds after one failure of the system.
struct HoldOnce {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    holds: Arc<AtomicUsize>,
}
impl Owner for HoldOnce {
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
        if self.holds.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(SystemDnsError::Backend)
        } else {
            Ok(())
        }
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
}

/// RC-1 M6 (G2; red on 83c5d3a2): a hold that failed on the system has an
/// owner: it re-proves itself, the plane sees one recovery, and its next
/// hold clears the fault.
#[tokio::test]
async fn a_failed_hold_is_recovered_by_its_owner_and_the_next_hold_clears_it() {
    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(HoldOnce {
        scopes,
        holds: Arc::new(AtomicUsize::new(0)),
    });
    let netmap = compiled().netmap.as_ref().clone();
    assert!(actor.hold(&netmap).await.is_err());
    assert_eq!(actor.state.borrow().epoch, 1, "one failure");
    let mut state = actor.state.clone();
    tokio::time::timeout(
        RECOVERY_INTERVAL * 3,
        state.wait_for(|fact| fact.fault.is_some() && fact.health.is_healthy()),
    )
    .await
    .expect("a failed hold has an owner that recovers it")
    .unwrap();
    actor.hold(&netmap).await.unwrap();
    assert_eq!(actor.state.borrow().fault, None, "the next hold clears it");
    actor.shutdown(true).await.unwrap();
}

/// An owner whose link settings someone else wrote.
struct ForeignSettings {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    recoveries: Arc<AtomicUsize>,
}
impl Owner for ForeignSettings {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Conflict)
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        self.recoveries.fetch_add(1, Ordering::SeqCst);
        Err(SystemDnsError::Conflict)
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        unreachable!("the plane does not hold a blocked owner")
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        unreachable!("the plane does not install on a blocked owner")
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
    fn failure(&self) -> Option<SystemDnsFailure> {
        Some(SystemDnsFailure::new(
            SystemDnsError::Conflict,
            "link_settings",
        ))
    }
}

/// RC-1 M6 (G3, Q2; red on 83c5d3a2): link settings someone else wrote are
/// proven: the owner is blocked and reads them again every 60 s, not every
/// second (its re-proof refuses foreign settings, never overwrites them).
#[tokio::test(start_paused = true)]
async fn foreign_link_settings_block_and_are_read_again_every_60_s() {
    let (scopes, _) = watch::channel(None);
    let recoveries = Arc::new(AtomicUsize::new(0));
    let actor = Actor::start(ForeignSettings {
        scopes,
        recoveries: recoveries.clone(),
    });
    let mut state = actor.state.clone();
    state.wait_for(|fact| fact.fault.is_some()).await.unwrap();
    assert!(state.borrow().health.is_blocked(), "proven, not retried");
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert_eq!(recoveries.load(Ordering::SeqCst), 0);
    tokio::time::sleep(Duration::from_secs(31)).await;
    assert_eq!(recoveries.load(Ordering::SeqCst), 1, "read again at 60 s");
    assert!(state.borrow().health.is_blocked());
    actor.shutdown(true).await.unwrap();
}

/// An owner whose observation failed once (a stalled process) while a
/// re-proof would still fail, and whose installation succeeds.
struct StalledOnce {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    observations: Arc<AtomicUsize>,
}
impl Owner for StalledOnce {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        if self.observations.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(SystemDnsError::Backend)
        } else {
            Ok(())
        }
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Backend)
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
}

/// RC-1 G15 (F7 `fault/throttle`: one more withdrawal at every restore): an
/// observation failed after the plane's hold, and the plane's installation
/// then succeeded. A successful installation ends with its own observation
/// and publishes query authority: it supersedes that fault (as a hold
/// does), so the plane does not withdraw again for a fault the installed
/// content already outlived.
#[tokio::test]
async fn a_successful_installation_supersedes_an_earlier_fault() {
    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(StalledOnce {
        scopes,
        observations: Arc::new(AtomicUsize::new(0)),
    });
    let mut state = actor.state.clone();
    state.wait_for(|fact| fact.fault.is_some()).await.unwrap();
    let compiled = std::sync::Arc::new(compiled());
    let desired = Desired::new(&compiled, &Default::default()).unwrap();
    actor.install(desired).await.unwrap();
    let fact = actor.state.borrow().clone();
    assert_eq!(fact.fault, None, "superseded by the installation");
    assert!(fact.health.is_healthy() && fact.detail.is_none());
    actor.shutdown(true).await.unwrap();
}
