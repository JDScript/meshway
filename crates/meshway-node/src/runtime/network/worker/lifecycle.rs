//! The fail-closed owners the network worker acts on (kernel ownership, the
//! DNS owner, the worker's own reinstall requests), consumed as edges.
//!
//! [`Healths::next`] wakes for a change of an owner's health it has not taken
//! yet, a due re-proof, or a queued reinstall, and marks it taken; it never
//! returns twice for the same fact. What the plane then does is [`decide`],
//! a pure function of the current [`Snapshot`] (not of the edge itself):
//! the same snapshot gives the same action, so every order of owner changes
//! is a test case of `decide`. A due re-proof comes first: no fact that stays
//! as it is while the worker cannot act on it may starve it.
use super::super::dns::DnsFact;
use meshway_model::recovery::Health;
use tokio::sync::watch;

/// What woke the worker.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Edge {
    /// The kernel ownership re-proof (or its slow re-read) is due.
    Due,
    /// Kernel ownership's health changed.
    Host,
    /// The DNS owner's fact changed.
    Dns,
    /// A queued reinstall of the same netmap (`trigger`).
    Reinstall(&'static str),
}

/// The DNS owner's fact as the plane reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DnsView {
    pub fault: bool,
    /// A fault the plane has not acknowledged yet (its epoch is later).
    pub unacknowledged: bool,
    /// The owner re-proved itself (or has no fault).
    pub healthy: bool,
}

impl DnsView {
    pub fn of(fact: &DnsFact, acknowledged: u64) -> Self {
        Self {
            fault: fact.fault.is_some(),
            unacknowledged: fact.epoch != acknowledged,
            healthy: fact.health.is_healthy(),
        }
    }
}

/// The current facts `decide` reads.
#[derive(Clone, Copy, Debug)]
pub(super) struct Snapshot {
    /// Kernel ownership ended (retrying or blocked).
    pub host_ended: bool,
    /// Its re-proof is due now.
    pub host_due: bool,
    pub dns: Option<DnsView>,
    /// The plane withdrew for the DNS owner's current fault.
    pub dns_failed: bool,
    /// Content is installed on the Host.
    pub installed: bool,
}

/// What the plane does for a snapshot.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    Nothing,
    /// Prove kernel ownership again.
    ReproveHost,
    /// The DNS owner failed: withdraw the plane.
    WithdrawDns,
    /// The failed DNS owner re-proved itself: install the same netmap.
    ReinstallDns,
    /// Kernel ownership ended under installed content: withdraw it.
    WithdrawHost,
}

/// Pure and idempotent: what the plane does now.
pub(super) fn decide(snapshot: &Snapshot) -> Action {
    if snapshot.host_ended && snapshot.host_due {
        return Action::ReproveHost;
    }
    if let Some(dns) = snapshot.dns
        && dns.fault
    {
        if !snapshot.dns_failed && (dns.unacknowledged || !dns.healthy) {
            return Action::WithdrawDns;
        }
        if snapshot.dns_failed && dns.healthy {
            return Action::ReinstallDns;
        }
    }
    if snapshot.host_ended && snapshot.installed {
        return Action::WithdrawHost;
    }
    Action::Nothing
}

/// The worker's subscriptions to every owner's health.
pub(super) struct Healths {
    host: Option<watch::Receiver<Health>>,
    dns: Option<watch::Receiver<DnsFact>>,
    /// A reinstall of the same netmap the worker queued for itself (an exit
    /// role whose prerequisites hold, a listener that bound or unbound, an
    /// overlap that ended): taken once.
    reinstall: Option<&'static str>,
}

impl Healths {
    pub fn new(host: Option<watch::Receiver<Health>>) -> Self {
        let host = host.map(|mut host| {
            host.mark_changed();
            host
        });
        Self {
            host,
            dns: None,
            reinstall: None,
        }
    }

    /// Follow the DNS owner the plane runs now (`None` while none runs): a
    /// new owner's first fact is one change.
    pub fn follow_dns(&mut self, current: Option<watch::Receiver<DnsFact>>) {
        match current {
            Some(current)
                if !self
                    .dns
                    .as_ref()
                    .is_some_and(|dns| dns.same_channel(&current)) =>
            {
                let mut current = current;
                current.mark_changed();
                self.dns = Some(current);
            }
            Some(_) => {}
            None => self.dns = None,
        }
    }

    /// Queue a reinstall of the same netmap; the first queued trigger stays.
    pub fn reinstall(&mut self, trigger: &'static str) {
        self.reinstall.get_or_insert(trigger);
    }

    /// Drop a queued reinstall (the netmap installs anyway).
    pub fn clear_reinstall(&mut self) {
        self.reinstall = None;
    }

    pub fn host(&self) -> Health {
        self.host
            .as_ref()
            .map(|host| host.borrow().clone())
            .unwrap_or_default()
    }

    pub fn dns(&self, acknowledged: u64) -> Option<DnsView> {
        self.dns
            .as_ref()
            .map(|dns| DnsView::of(&dns.borrow(), acknowledged))
    }

    /// The next fact not taken yet: a due re-proof first, then a queued
    /// reinstall, kernel ownership and the DNS owner. Each change is taken
    /// once; a closed subscription is dropped.
    pub async fn next(&mut self, due: Option<tokio::time::Instant>) -> Edge {
        if due.is_some_and(|due| due <= tokio::time::Instant::now()) {
            return Edge::Due;
        }
        if let Some(trigger) = self.reinstall.take() {
            return Edge::Reinstall(trigger);
        }
        loop {
            let host = async {
                match self.host.as_mut() {
                    Some(host) => host.changed().await.is_ok(),
                    None => std::future::pending().await,
                }
            };
            let dns = async {
                match self.dns.as_mut() {
                    Some(dns) => dns.changed().await.is_ok(),
                    None => std::future::pending().await,
                }
            };
            tokio::select! { biased;
                _ = super::sleep_until(due) => return Edge::Due,
                open = host => {
                    if !open { self.host = None; continue; }
                    if let Some(host) = &mut self.host { host.borrow_and_update(); }
                    return Edge::Host;
                },
                open = dns => {
                    if !open { self.dns = None; continue; }
                    if let Some(dns) = &mut self.dns { dns.borrow_and_update(); }
                    return Edge::Dns;
                },
            }
        }
    }

    /// Completes once an owner's health changed since it was last taken
    /// (the packet drive yields to it), without taking it.
    pub fn changed(&self) -> impl std::future::Future<Output = ()> + use<> {
        let mut host = self.host.clone();
        let mut dns = self.dns.clone();
        async move {
            let host = async {
                match host.as_mut() {
                    Some(host) => {
                        let _ = host.changed().await;
                    }
                    None => std::future::pending().await,
                }
            };
            let dns = async {
                match dns.as_mut() {
                    Some(dns) => {
                        let _ = dns.changed().await;
                    }
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = host => {},
                _ = dns => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshway_dns::SystemDnsError;
    use meshway_model::recovery::Retrying;
    use std::time::Duration;

    fn retrying() -> Health {
        Health::Retrying(Retrying {
            check: "observe_deadline",
            reason: "stalled".into(),
            since: std::time::Instant::now(),
            attempts: 1,
            retry_at: std::time::Instant::now(),
        })
    }

    /// The DNS owner's fact, driven as its actor drives it.
    struct Owner {
        state: watch::Sender<DnsFact>,
    }
    impl Owner {
        fn new() -> Self {
            Self {
                state: watch::channel(DnsFact::default()).0,
            }
        }
        fn fail(&self, error: SystemDnsError) {
            self.state.send_modify(|fact| {
                if fact.fault.is_none() || fact.health.is_healthy() {
                    fact.epoch += 1;
                }
                fact.fault = Some(error);
                fact.health = retrying();
            });
        }
        fn recover(&self) {
            self.state.send_modify(|fact| fact.health = Health::Healthy);
        }
        fn hold(&self) {
            self.state.send_modify(|fact| fact.fault = None);
        }
        fn epoch(&self) -> u64 {
            self.state.borrow().epoch
        }
    }

    fn snapshot(
        healths: &Healths,
        acknowledged: u64,
        dns_failed: bool,
        installed: bool,
        due: bool,
    ) -> Snapshot {
        let host = healths.host();
        Snapshot {
            host_ended: !host.is_healthy(),
            host_due: due,
            dns: healths.dns(acknowledged),
            dns_failed,
            installed,
        }
    }

    /// X15: a due re-proof is decided before DNS facts that stay as they
    /// are, failed or re-proved; not due, the DNS facts come in order.
    #[tokio::test(start_paused = true)]
    async fn a_due_reproof_is_selected_before_dns_facts_that_stay_ready() {
        let owner = Owner::new();
        let (host, ownership) = watch::channel(retrying());
        let mut healths = Healths::new(Some(ownership));
        healths.follow_dns(Some(owner.state.subscribe()));
        owner.fail(SystemDnsError::Backend);
        let due = Some(tokio::time::Instant::now());
        assert_eq!(healths.next(due).await, Edge::Due);
        assert_eq!(
            decide(&snapshot(&healths, 0, false, true, true)),
            Action::ReproveHost
        );
        owner.recover();
        assert_eq!(
            decide(&snapshot(&healths, owner.epoch(), true, true, true)),
            Action::ReproveHost
        );
        // Not due yet: the facts come in their order.
        let later = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        let mut edges = Vec::new();
        for _ in 0..2 {
            edges.push(healths.next(later).await);
        }
        assert_eq!(edges, [Edge::Host, Edge::Dns]);
        assert_eq!(
            decide(&snapshot(&healths, owner.epoch(), true, true, false)),
            Action::ReinstallDns
        );
        drop(host);
    }

    /// S4 (OPS16, planck 17:59:57): the process stalled 4.5 s; the owner's
    /// observation failed and its recovery succeeded 7 ms later, before the
    /// plane looked. Unacknowledged, that fault is still one failure and then
    /// one recovery; acknowledged and re-proved, it waits for the hold.
    #[tokio::test(start_paused = true)]
    async fn a_fault_re_proved_before_the_plane_looked_is_one_failure_and_one_recovery() {
        let owner = Owner::new();
        let mut healths = Healths::new(None);
        healths.follow_dns(Some(owner.state.subscribe()));
        assert_eq!(healths.next(None).await, Edge::Dns);
        assert_eq!(
            decide(&snapshot(&healths, 0, false, true, false)),
            Action::Nothing
        );
        owner.fail(SystemDnsError::Backend);
        owner.recover();
        assert_eq!(healths.next(None).await, Edge::Dns);
        assert_eq!(
            decide(&snapshot(&healths, 0, false, true, false)),
            Action::WithdrawDns
        );
        let acknowledged = owner.epoch();
        assert_eq!(
            decide(&snapshot(&healths, acknowledged, true, false, false)),
            Action::ReinstallDns
        );
        // The reinstall did not reach the hold: no second failure, and the
        // worker sleeps (no edge).
        assert_eq!(
            decide(&snapshot(&healths, acknowledged, false, false, false)),
            Action::Nothing
        );
        let idle = tokio::time::timeout(Duration::from_secs(10), healths.next(None)).await;
        assert!(idle.is_err(), "no edge without a change");
        // The hold clears it; a later fault is a new failure, re-proved or not.
        owner.hold();
        owner.fail(SystemDnsError::Conflict);
        owner.recover();
        assert_eq!(healths.next(None).await, Edge::Dns);
        assert_eq!(
            decide(&snapshot(&healths, acknowledged, false, true, false)),
            Action::WithdrawDns
        );
    }

    /// OPS14 x13-1/x13-2: the DNS owner failed and re-proved itself, then
    /// kernel ownership ended unavailable before the plane's reinstall could
    /// reach the DNS hold. The worker reacts to the recovery once and then
    /// waits; the re-proof due 1 s later is selected within a bounded number
    /// of turns (alpha.15: ~37 000 turns/s, never selected).
    #[tokio::test(start_paused = true)]
    async fn a_recovered_dns_fault_and_an_ended_term_reach_the_reproof_without_spinning() {
        let owner = Owner::new();
        let (host, ownership) = watch::channel(Health::Healthy);
        let mut healths = Healths::new(Some(ownership));
        healths.follow_dns(Some(owner.state.subscribe()));
        let started = tokio::time::Instant::now();
        let mut dns_failed = false;
        let mut acknowledged = 0;
        let mut installed = true;
        owner.fail(SystemDnsError::Backend);
        owner.recover();
        host.send_replace(retrying());
        let due = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        let mut turns = 0;
        let mut actions = Vec::new();
        loop {
            turns += 1;
            assert!(turns <= 8, "the worker spins: {actions:?}");
            let edge = tokio::time::timeout(Duration::from_secs(10), healths.next(due))
                .await
                .expect("an edge within 10 s");
            let now_due = due.is_some_and(|due| tokio::time::Instant::now() >= due);
            let action = decide(&snapshot(
                &healths,
                acknowledged,
                dns_failed,
                installed,
                now_due,
            ));
            match action {
                Action::WithdrawDns => {
                    dns_failed = true;
                    acknowledged = owner.epoch();
                    installed = false;
                }
                // The reinstall fails at the ended capture before the DNS
                // hold: the owner's fault and recovery stay as they are.
                Action::ReinstallDns => dns_failed = false,
                Action::WithdrawHost => installed = false,
                Action::ReproveHost => break,
                Action::Nothing => {}
            }
            actions.push((edge, action));
        }
        assert!(started.elapsed() <= Duration::from_secs(10));
        assert!(
            actions
                .iter()
                .filter(|(_, action)| *action == Action::ReinstallDns)
                .count()
                == 1,
            "one recovery: {actions:?}"
        );
    }

    /// RC-1 M7 (G8): kernel ownership ending unavailable and then becoming
    /// a proven conflict before any re-proof are two edges: the worker (and
    /// Status, which reads the same health) sees the upgrade, though no
    /// content is installed and no re-proof is due.
    #[tokio::test(start_paused = true)]
    async fn an_unavailable_end_upgraded_to_a_conflict_is_an_edge() {
        let (host, ownership) = watch::channel(Health::Healthy);
        let mut healths = Healths::new(Some(ownership));
        assert_eq!(healths.next(None).await, Edge::Host);
        host.send_replace(retrying());
        assert_eq!(healths.next(None).await, Edge::Host);
        host.send_replace(Health::Blocked(meshway_model::recovery::Blocked {
            check: "monitor",
            reason: "foreign rule".into(),
            since: std::time::Instant::now(),
            attempts: 1,
            reread_at: None,
        }));
        let edge = tokio::time::timeout(Duration::from_secs(1), healths.next(None)).await;
        assert_eq!(edge.ok(), Some(Edge::Host), "the upgrade is an edge");
        assert!(healths.host().is_blocked());
        assert_eq!(
            decide(&snapshot(&healths, 0, false, false, false)),
            Action::Nothing,
            "withdrawn already: nothing to do but report"
        );
        // A queued reinstall is taken once.
        healths.reinstall("exitRecovered");
        healths.reinstall("listenersChanged");
        assert_eq!(healths.next(None).await, Edge::Reinstall("exitRecovered"));
        let idle = tokio::time::timeout(Duration::from_secs(1), healths.next(None)).await;
        assert!(idle.is_err());
    }
}
