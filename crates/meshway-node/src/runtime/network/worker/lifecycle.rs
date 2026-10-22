//! Component lifecycle facts the network worker selects on, in priority
//! order. A due re-proof of kernel ownership comes first: no fact that stays
//! ready while the worker cannot act on it may starve it. The DNS facts are
//! edges (see [`Health`]): each fault is one failure and at most one
//! recovery, whatever the reinstall it triggers reaches.
use super::super::dns::DnsHealth as Health;
use std::future::Future;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Lifecycle {
    /// An unavailable end of kernel ownership is due for its re-proof.
    HostRecovery,
    DnsFailure,
    /// The failed DNS owner re-proved its system installation.
    DnsRecovered,
    HostFailed,
}

/// `dns_failed`: the plane withdrew for the DNS owner's current fault and
/// waits for its recovery. `installed`: content is installed on the Host,
/// so the end of its ownership withdraws it.
pub(super) async fn next(
    recovery_at: Option<tokio::time::Instant>,
    dns: Option<Health>,
    dns_failed: bool,
    host_ended: impl Future<Output = ()>,
    installed: bool,
) -> Lifecycle {
    let failed = dns.clone();
    let recovered = dns;
    tokio::select! { biased;
        _ = super::sleep_until(recovery_at) => Lifecycle::HostRecovery,
        _ = async { match failed { Some(health) => health.failed().await, None => std::future::pending().await } }, if !dns_failed => Lifecycle::DnsFailure,
        _ = async { match recovered { Some(health) => health.recovered().await, None => std::future::pending().await } }, if dns_failed => Lifecycle::DnsRecovered,
        _ = host_ended, if installed => Lifecycle::HostFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::dns::DnsFact;
    use super::*;
    use meshway_dns::SystemDnsError;
    use std::time::Duration;
    use tokio::sync::watch;

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
        /// A new fault, retried by the owner.
        fn fail(&self, error: SystemDnsError) {
            self.state.send_modify(|fact| {
                if fact.fault.is_none() || fact.health.is_healthy() {
                    fact.epoch += 1;
                }
                fact.fault = Some(error);
                fact.health =
                    meshway_model::recovery::Health::Retrying(meshway_model::recovery::Retrying {
                        check: "observe_deadline",
                        reason: "stalled".into(),
                        since: std::time::Instant::now(),
                        attempts: 1,
                        retry_at: std::time::Instant::now(),
                    });
            });
        }
        /// The owner re-proved its installation.
        fn recover(&self) {
            self.state
                .send_modify(|fact| fact.health = meshway_model::recovery::Health::Healthy);
        }
        /// The plane's hold cleared the fault.
        fn hold(&self) {
            self.state.send_modify(|fact| fact.fault = None);
        }
        /// As the plane sees it after acknowledging every fault so far.
        fn health(&self) -> Health {
            self.health_acknowledging(self.epoch())
        }
        fn health_acknowledging(&self, acknowledged: u64) -> Health {
            Health::new(self.state.subscribe(), acknowledged)
        }
        fn epoch(&self) -> u64 {
            self.state.borrow().epoch
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_due_reproof_is_selected_before_dns_facts_that_stay_ready() {
        let owner = Owner::new();
        owner.fail(SystemDnsError::Backend);
        let due = Some(tokio::time::Instant::now());
        // Failed and not yet withdrawn for: ready, still after the re-proof.
        assert_eq!(
            next(
                due,
                Some(owner.health()),
                false,
                std::future::ready(()),
                true
            )
            .await,
            Lifecycle::HostRecovery
        );
        owner.recover();
        assert_eq!(
            next(
                due,
                Some(owner.health()),
                true,
                std::future::ready(()),
                true
            )
            .await,
            Lifecycle::HostRecovery
        );
        // Not due yet: the facts come in their order.
        let later = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        assert_eq!(
            next(
                later,
                Some(owner.health()),
                true,
                std::future::ready(()),
                true
            )
            .await,
            Lifecycle::DnsRecovered
        );
    }

    /// S4 (OPS16, planck 17:59:57): the process stalled 4.5 s; the owner's
    /// observation failed and its recovery succeeded 7 ms later, before the
    /// plane looked. Unacknowledged, that fault is still one failure and then
    /// one recovery (alpha.16 saw neither: the plane kept its content while
    /// DNS query authority stayed withdrawn and Status read `suspended`).
    /// Acknowledged and re-proved, it waits for the plane's hold.
    #[tokio::test(start_paused = true)]
    async fn a_fault_re_proved_before_the_plane_looked_is_one_failure_and_one_recovery() {
        let owner = Owner::new();
        let mut acknowledged = owner.epoch();
        let health = |acknowledged: u64| owner.health_acknowledging(acknowledged);
        let within = |next| tokio::time::timeout(Duration::from_secs(10), next);
        owner.fail(SystemDnsError::Backend);
        owner.recover();
        let failed = within(next(
            None,
            Some(health(acknowledged)),
            false,
            std::future::pending(),
            true,
        ))
        .await;
        assert_eq!(failed.ok(), Some(Lifecycle::DnsFailure));
        acknowledged = owner.epoch();
        let recovered = within(next(
            None,
            Some(health(acknowledged)),
            true,
            std::future::pending(),
            false,
        ))
        .await;
        assert_eq!(recovered.ok(), Some(Lifecycle::DnsRecovered));
        acknowledged = owner.epoch();
        // The reinstall did not reach the hold: no second failure.
        let idle = within(next(
            None,
            Some(health(acknowledged)),
            false,
            std::future::pending(),
            false,
        ))
        .await;
        assert!(
            idle.is_err(),
            "an acknowledged re-proved fault is no failure"
        );
        // The hold clears it; a later fault is a new failure, re-proved or not.
        owner.hold();
        owner.fail(SystemDnsError::Conflict);
        owner.recover();
        let again = within(next(
            None,
            Some(health(acknowledged)),
            false,
            std::future::pending(),
            true,
        ))
        .await;
        assert_eq!(again.ok(), Some(Lifecycle::DnsFailure));
    }

    /// OPS14 x13-1/x13-2: the DNS owner failed and re-proved itself, then
    /// kernel ownership ended unavailable before the plane's reinstall could
    /// reach the DNS hold (the capture installation fails first and the
    /// recovered fault stays). The worker reacts to the recovery once and
    /// then waits; the re-proof due 1 s later is selected, within 10 s and a
    /// bounded number of turns (alpha.15: ~37 000 turns/s, never selected).
    #[tokio::test(start_paused = true)]
    async fn a_recovered_dns_fault_and_an_ended_term_reach_the_reproof_without_spinning() {
        let owner = Owner::new();
        let started = tokio::time::Instant::now();
        let mut dns_failed = false;
        owner.fail(SystemDnsError::Backend);
        assert_eq!(
            next(
                None,
                Some(owner.health()),
                dns_failed,
                std::future::pending(),
                true
            )
            .await,
            Lifecycle::DnsFailure
        );
        dns_failed = true;
        owner.recover();
        // Ownership ends now; its re-proof is due 1 s later.
        let recovery_at = Some(tokio::time::Instant::now() + Duration::from_secs(1));
        let mut turns = 0;
        let selected = loop {
            turns += 1;
            assert!(turns <= 4, "the worker spins");
            match tokio::time::timeout(
                Duration::from_secs(10),
                next(
                    recovery_at,
                    Some(owner.health()),
                    dns_failed,
                    // Withdrawn: no content installed on the ended term.
                    std::future::pending(),
                    false,
                ),
            )
            .await
            .expect("the re-proof is selected within 10 s")
            {
                // The reinstall fails at the ended capture before the DNS
                // hold: the owner's fault and recovery stay as they are.
                Lifecycle::DnsRecovered => dns_failed = false,
                Lifecycle::DnsFailure => dns_failed = true,
                other => break other,
            }
        };
        assert_eq!(selected, Lifecycle::HostRecovery);
        assert_eq!(turns, 2, "one recovery, then the re-proof");
        assert!(started.elapsed() <= Duration::from_secs(10));
        // A new fault after the plane's hold is a new failure.
        owner.hold();
        owner.fail(SystemDnsError::Conflict);
        assert_eq!(
            next(
                None,
                Some(owner.health()),
                dns_failed,
                std::future::pending(),
                true
            )
            .await,
            Lifecycle::DnsFailure
        );
    }
}
