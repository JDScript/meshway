use super::*;
use meshway_control::connectivity::DiscoveryContext;
use meshway_model::state::{EndpointCandidate, EndpointSource};
use meshway_stun::{Fingerprint, TransactionId};

pub(super) struct Pending {
    context: DiscoveryContext,
    remote: SocketAddr,
    transaction: TransactionId,
    output: Vec<u8>,
    deadline: Instant,
    retry: Retry,
    /// When the transaction was last sent, and how often.
    sent: Option<Instant>,
    sends: u8,
}

#[derive(Debug)]
struct Lifetime {
    context: DiscoveryContext,
    scope: Scope,
    deadline: Instant,
}
impl meshway_transport::SendLifetime for Lifetime {
    fn is_current(&self) -> bool {
        self.context.ensure_current().is_ok()
            && self.scope.check().is_ok()
            && Instant::now() < self.deadline
    }
    fn ended(self: Arc<Self>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            tokio::select! {
                _ = self.context.ended() => {},
                _ = tokio::time::sleep_until(self.deadline) => {},
                _ = self.scope.run(std::future::pending::<Result<(), Error>>()) => {},
            }
        })
    }
}

impl Owner {
    pub(super) fn discover(
        &mut self,
        context: DiscoveryContext,
        server: String,
        targets: Vec<SocketAddr>,
    ) -> Result<(), Error> {
        if context.ensure_current().is_err()
            || !context.servers().iter().any(|value| value.id == server)
            || context.netmap() != self.shared.plan().reference
        {
            return Ok(());
        }
        for remote in targets.into_iter().take(2) {
            if self.discovery.len() >= 8
                || self
                    .discovery
                    .values()
                    .any(|pending| pending.remote == remote)
                || remote.is_ipv4() != self.local.is_ipv4()
            {
                continue;
            }
            let transaction = TransactionId::random().map_err(|_| Error::Authentication)?;
            // Reservation precedes the first Sender admission, including a
            // loopback-speed response; there is still only one recv owner.
            self.discovery.insert(
                *transaction.as_bytes(),
                Pending {
                    context: context.clone(),
                    remote,
                    transaction,
                    output: meshway_stun::encode_request(transaction, Fingerprint::Include),
                    deadline: Instant::now()
                        .checked_add(Duration::from_secs(5))
                        .ok_or(Error::Limits)?
                        .min(context.expires_at().into()),
                    retry: Retry::due(),
                    sent: None,
                    sends: 0,
                },
            );
        }
        Ok(())
    }

    pub(super) async fn stun_tick(&mut self) -> Result<(), Error> {
        self.discovery.retain(|_, pending| {
            pending.context.ensure_current().is_ok() && Instant::now() < pending.deadline
        });
        for pending in self.discovery.values_mut() {
            if !pending.retry.take(Duration::from_millis(500), MAX_SENDS) {
                continue;
            }
            pending.sent = Some(Instant::now());
            pending.sends = pending.sends.saturating_add(1);
            let lifetime = Arc::new(Lifetime {
                context: pending.context.clone(),
                scope: self.lifetime.clone(),
                deadline: pending.deadline,
            });
            let _ = traversal::send(
                &self.sender,
                lifetime,
                pending.deadline,
                pending.remote,
                pending.output.clone(),
            )
            .await;
        }
        let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
        let changed = {
            let mut sockets = self.shared.sockets.lock().map_err(|_| Error::Worker)?;
            if let Some(fact) = sockets.get_mut(&self.id) {
                let before = fact.endpoints.len();
                fact.endpoints.retain(|value| {
                    u64::try_from(value.expires_at.timestamp_millis())
                        .is_ok_and(|expires| now < expires)
                });
                fact.endpoints.len() != before
            } else {
                false
            }
        };
        if changed {
            self.changed();
        }
        Ok(())
    }

    pub(super) fn stun_receive(&mut self, remote: SocketAddr, bytes: &[u8]) -> Result<(), Error> {
        let Some(transaction) = bytes
            .get(8..20)
            .and_then(|bytes| <[u8; 12]>::try_from(bytes).ok())
        else {
            return Ok(());
        };
        let Some(pending) = self.discovery.get(&transaction) else {
            return Ok(());
        };
        if pending.context.ensure_current().is_err() || Instant::now() >= pending.deadline {
            return Ok(());
        }
        let Ok(mapped) =
            meshway_stun::parse_response(pending.transaction, pending.remote, remote, bytes)
        else {
            return Ok(());
        };
        let address = mapped.socket_addr();
        if address.is_ipv4() != self.local.is_ipv4()
            || meshway_control::connectivity::udp_recipient(&address.to_string()).is_err()
        {
            return Ok(());
        }
        let now = pending.context.now_ms().map_err(|_| Error::Authority)?;
        let timestamp = |ms| {
            i64::try_from(ms)
                .ok()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .ok_or(Error::Authority)
        };
        // A mapping observation, not authority: its lifetime is the NAT
        // binding horizon, independent of the metadata context that
        // authorized the request.
        let observation = EndpointCandidate {
            address: address.to_string(),
            origin: EndpointSource::Stun {
                // State records the actual validated observer address, not the
                // metadata selection ID (one ID may resolve to multiple peers).
                observer: remote.to_string(),
            },
            observed_at: timestamp(now)?,
            expires_at: timestamp(now.saturating_add(OBSERVATION_LIFETIME_MS))?,
        };
        let mut sockets = self.shared.sockets.lock().map_err(|_| Error::Worker)?;
        let changed = sockets
            .get_mut(&self.id)
            .is_some_and(|fact| record(&mut fact.endpoints, observation, now));
        drop(sockets);
        if let Ok(mut confirmed) = self.shared.stun_confirmed.lock() {
            confirmed.insert(self.id, Instant::now());
        }
        // A retransmitted transaction has no unambiguous round trip.
        let rtt = pending
            .sent
            .filter(|_| pending.sends == 1)
            .map(|sent| sent.elapsed());
        if let Ok(mut samples) = self.shared.stun_samples.lock() {
            let observers = samples.entry(self.id).or_default();
            if observers.len() < 8 || observers.contains_key(&remote) {
                observers.insert(
                    remote,
                    crate::manager::StunSample {
                        mapped: address,
                        rtt,
                        at: Instant::now(),
                    },
                );
            }
        }
        self.discovery.remove(&transaction);
        if changed {
            self.changed();
        }
        Ok(())
    }
}

/// Reported lifetime of one STUN observation.
pub(in crate::manager) const OBSERVATION_LIFETIME_MS: u64 = 120_000;
/// An unchanged mapping is re-issued only once its observation has less
/// than this left, so a confirmation is not a State report.
const RENEW_BEFORE_MS: u64 = 60_000;

/// Fold one validated observation into a socket's endpoints. A new mapping
/// replaces this observer's observations; an unchanged one only renews near
/// expiry and keeps its previous observation, which offers prepared from the
/// last acknowledged report may still name, until that expires on its own.
/// Returns whether the reported endpoints changed.
fn record(
    endpoints: &mut Vec<EndpointCandidate>,
    observation: EndpointCandidate,
    now: u64,
) -> bool {
    let expiry = |candidate: &EndpointCandidate| {
        u64::try_from(candidate.expires_at.timestamp_millis()).unwrap_or(0)
    };
    let latest = endpoints
        .iter()
        .filter(|old| old.origin == observation.origin && old.address == observation.address)
        .map(expiry)
        .max();
    if latest.is_some_and(|expires| expires > now.saturating_add(RENEW_BEFORE_MS)) {
        return false;
    }
    endpoints.retain(|old| {
        old.origin != observation.origin
            || (old.address == observation.address
                && Some(expiry(old)) == latest
                && expiry(old) > now)
    });
    if endpoints.len() < 8 {
        endpoints.push(observation);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(ms as i64).unwrap()
    }
    fn observed(address: &str, observer: &str, ms: u64) -> EndpointCandidate {
        EndpointCandidate {
            address: address.into(),
            origin: EndpointSource::Stun {
                observer: observer.into(),
            },
            observed_at: at(ms),
            expires_at: at(ms + OBSERVATION_LIFETIME_MS),
        }
    }

    #[test]
    fn an_unchanged_mapping_is_reported_only_when_its_observation_nears_expiry() {
        let mut endpoints = Vec::new();
        let base = 1_000_000;
        assert!(record(
            &mut endpoints,
            observed("192.0.2.1:9", "198.51.100.1:3478", base),
            base
        ));
        // Confirmations at the 10 s / 30 s cadence change nothing reported.
        for elapsed in [10_000, 30_000, 59_000] {
            let now = base + elapsed;
            assert!(!record(
                &mut endpoints,
                observed("192.0.2.1:9", "198.51.100.1:3478", now),
                now
            ));
        }
        assert_eq!(
            endpoints,
            vec![observed("192.0.2.1:9", "198.51.100.1:3478", base)]
        );
        // Renewal keeps the previous observation beside the new one.
        let renew = base + 61_000;
        assert!(record(
            &mut endpoints,
            observed("192.0.2.1:9", "198.51.100.1:3478", renew),
            renew
        ));
        assert_eq!(
            endpoints,
            vec![
                observed("192.0.2.1:9", "198.51.100.1:3478", base),
                observed("192.0.2.1:9", "198.51.100.1:3478", renew)
            ]
        );
        // The next renewal drops the oldest; at most two per observer.
        let again = renew + 61_000;
        assert!(record(
            &mut endpoints,
            observed("192.0.2.1:9", "198.51.100.1:3478", again),
            again
        ));
        assert_eq!(
            endpoints,
            vec![
                observed("192.0.2.1:9", "198.51.100.1:3478", renew),
                observed("192.0.2.1:9", "198.51.100.1:3478", again)
            ]
        );
    }

    #[test]
    fn a_changed_mapping_replaces_the_observer_at_once_and_keeps_other_observers() {
        let base = 1_000_000;
        let mut endpoints = vec![
            observed("192.0.2.1:9", "198.51.100.1:3478", base),
            observed("192.0.2.1:9", "203.0.113.1:3478", base),
        ];
        let now = base + 10_000;
        assert!(record(
            &mut endpoints,
            observed("192.0.2.7:4000", "198.51.100.1:3478", now),
            now
        ));
        assert_eq!(
            endpoints,
            vec![
                observed("192.0.2.1:9", "203.0.113.1:3478", base),
                observed("192.0.2.7:4000", "198.51.100.1:3478", now)
            ]
        );
    }
}
