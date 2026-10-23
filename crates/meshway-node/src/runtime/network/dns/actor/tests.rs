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

/// F8a F6: a failed preparation (no owner runs yet) is judged on its full
/// failure as the running owner's are and published for Status: proven is
/// `blocked`, prepared again in 60 s; anything else `retrying`.
#[tokio::test]
async fn a_failed_preparation_is_judged_and_published_as_a_running_owners_fault() {
    let mut attachment = Attachment::new(Default::default());
    assert!(attachment.report_guard().is_none() && attachment.prepare_at().is_none());
    let proven = SystemDnsFailure::new(SystemDnsError::Conflict, "owner_record");
    assert!(proven.proven());
    let started = tokio::time::Instant::now();
    assert!(attachment.prepared(Err(proven.clone())).is_err());
    let guard = attachment.report_guard().expect("the failure is published");
    assert!(
        guard.borrow().health.is_blocked(),
        "{:?}",
        guard.borrow().health
    );
    assert_eq!(guard.borrow().fault, Some(SystemDnsError::Conflict));
    assert_eq!(guard.borrow().detail.as_ref(), Some(&proven));
    assert!(attachment.prepare_at().unwrap() >= started + REREAD_PROVEN);
    assert!(
        attachment.guard().is_none(),
        "the plane acts on a running owner only"
    );

    let mut attachment = Attachment::new(Default::default());
    let unknown = SystemDnsFailure::new(SystemDnsError::Storage, "reserve");
    assert!(attachment.prepared(Err(unknown)).is_err());
    let guard = attachment.report_guard().unwrap();
    let health = guard.borrow().health.clone();
    assert!(matches!(health, Health::Retrying(_)), "{health:?}");
    let at = attachment.prepare_at().unwrap();
    assert!(at >= started + PREPARE_RETRY && at < started + REREAD_PROVEN);
}

/// D5: the installer's coexisting local domains reach the one DNS fact
/// after a successful observation, change it once, and leave it with a
/// fault (unknown then).
struct LocalOwner {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    observations: Arc<AtomicUsize>,
    domains: Arc<std::sync::Mutex<Vec<meshway_protocol::ipc::LocalDnsDomain>>>,
    fail_after: usize,
}

impl Owner for LocalOwner {
    fn scopes(&self) -> watch::Sender<Option<meshway_dns::Scope>> {
        self.scopes.clone()
    }
    async fn observe(&mut self) -> Result<(), SystemDnsError> {
        if self.observations.fetch_add(1, Ordering::SeqCst) >= self.fail_after {
            Err(SystemDnsError::Backend)
        } else {
            Ok(())
        }
    }
    async fn recover(&mut self) -> Result<(), SystemDnsError> {
        Err(SystemDnsError::Backend)
    }
    async fn hold(&mut self, _: &NodeNetmap) -> Result<(), SystemDnsError> {
        unreachable!("this regression covers observation only")
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        unreachable!("this regression covers observation only")
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
    fn local_domains(&self) -> Vec<meshway_protocol::ipc::LocalDnsDomain> {
        self.domains.lock().unwrap().clone()
    }
}

#[tokio::test(start_paused = true)]
async fn local_domains_follow_successful_observations_and_clear_with_a_fault() {
    let (scopes, _) = watch::channel(None);
    let observations = Arc::new(AtomicUsize::new(0));
    let local = meshway_protocol::ipc::LocalDnsDomain {
        domain: "tko.oraclevcn.com".into(),
        link_index: 2,
        link_name: Some("enp0s6".into()),
    };
    let domains = Arc::new(std::sync::Mutex::new(vec![local.clone()]));
    let actor = Actor::start(LocalOwner {
        scopes,
        observations: observations.clone(),
        domains: domains.clone(),
        fail_after: 6,
    });
    let mut state = actor.state.clone();
    state.mark_unchanged();
    tokio::time::timeout(Duration::from_secs(1), state.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.borrow_and_update().local_domains, vec![local.clone()]);
    // The same domains on the next observations send nothing.
    while observations.load(Ordering::SeqCst) < 4 {
        tokio::time::sleep(super::super::OBSERVE_INTERVAL).await;
    }
    assert!(!state.has_changed().unwrap());
    domains.lock().unwrap().clear();
    tokio::time::timeout(Duration::from_secs(1), state.changed())
        .await
        .unwrap()
        .unwrap();
    assert!(state.borrow_and_update().local_domains.is_empty());
    domains.lock().unwrap().push(local);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            state.changed().await.unwrap();
            let fact = state.borrow_and_update();
            if fact.fault.is_some() {
                assert!(fact.local_domains.is_empty());
                return;
            }
        }
    })
    .await
    .unwrap();
    actor.shutdown(true).await.unwrap();
}

/// An owner whose installation fails as scripted: `probe` (the resolved
/// backend's probe found no query reaching the bind), another system
/// failure, or success. Every re-proof holds.
struct ScriptedHold {
    scopes: watch::Sender<Option<meshway_dns::Scope>>,
    script: Arc<std::sync::Mutex<std::collections::VecDeque<Option<SystemDnsFailure>>>>,
    last: Option<SystemDnsFailure>,
}
impl Owner for ScriptedHold {
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
        self.last = self.script.lock().unwrap().pop_front().unwrap();
        match &self.last {
            Some(failure) => Err(failure.error),
            None => Ok(()),
        }
    }
    async fn install(&mut self, _: Desired) -> Result<(), SystemDnsError> {
        Ok(())
    }
    async fn shutdown(self, _: bool) -> Result<(), SystemDnsError> {
        Ok(())
    }
    fn failure(&self) -> Option<SystemDnsFailure> {
        self.last.clone()
    }
}

/// DNS-2b (red before: every failed probe waited the fixed 1 s, so a host
/// without a usable resolved rewrote the anchor link every ~6 s): repeated
/// `probe` failures wait 1, 2, 4 … s, capped at 60 s, each re-proof
/// holding; a successful hold resets the pause; another system failure
/// keeps the fixed 1 s and ends the count.
#[tokio::test(start_paused = true)]
async fn a_repeated_probe_failure_backs_off_to_60_s_and_a_success_resets_it() {
    let probe = || {
        SystemDnsFailure::new(SystemDnsError::Unsupported, "probe")
            .expected("resolved sends a nonce query to the owned link's loopback DNS server")
            .actual("no query reached 127.0.0.1:41000 within 5 s")
    };
    let other = || SystemDnsFailure::new(SystemDnsError::Backend, "resolved_call");
    let mut script = std::collections::VecDeque::new();
    script.extend((0..9).map(|_| Some(probe())));
    script.push_back(None);
    script.extend((0..2).map(|_| Some(probe())));
    script.extend((0..2).map(|_| Some(other())));
    script.push_back(Some(probe()));
    let script = Arc::new(std::sync::Mutex::new(script));
    let (scopes, _) = watch::channel(None);
    let actor = Actor::start(ScriptedHold {
        scopes,
        script: script.clone(),
        last: None,
    });
    let netmap = compiled().netmap.as_ref().clone();
    let mut state = actor.state.clone();
    let mut waits = Vec::new();
    while !script.lock().unwrap().is_empty() {
        let failed = actor.hold(&netmap).await.is_err();
        if !failed {
            continue;
        }
        let failed_at = tokio::time::Instant::now();
        let Health::Retrying(retrying) = state.borrow().health.clone() else {
            panic!("a failed hold retries");
        };
        waits.push((retrying.retry_at - failed_at.into_std()).as_secs());
        // The plane holds again once the owner re-proved itself.
        tokio::time::timeout(
            Duration::from_secs(120),
            state.wait_for(|fact| fact.fault.is_some() && fact.health.is_healthy()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            tokio::time::Instant::now() - failed_at >= Duration::from_secs(waits[waits.len() - 1])
        );
    }
    assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60, 1, 2, 1, 1, 1]);
    actor.shutdown(true).await.unwrap();
}
