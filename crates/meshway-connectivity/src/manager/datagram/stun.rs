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
    fn invalidated(
        self: Arc<Self>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            tokio::select! {
                _ = self.context.invalidated() => {},
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
            || !context
                .snapshot()
                .same_netmap(self.shared.identity.snapshot())
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
        let now = self
            .shared
            .identity
            .snapshot()
            .now_ms()
            .map_err(|_| Error::Authority)?;
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
        let now = pending
            .context
            .snapshot()
            .now_ms()
            .map_err(|_| Error::Authority)?;
        let expires = now
            .saturating_add(20_000)
            .min(pending.context.expires_at_ms());
        let timestamp = |ms| {
            i64::try_from(ms)
                .ok()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .ok_or(Error::Authority)
        };
        let observation = EndpointCandidate {
            address: address.to_string(),
            origin: EndpointSource::Stun {
                // State records the actual validated observer address, not the
                // metadata selection ID (one ID may resolve to multiple peers).
                observer: remote.to_string(),
            },
            observed_at: timestamp(now)?,
            expires_at: timestamp(expires)?,
        };
        let mut sockets = self.shared.sockets.lock().map_err(|_| Error::Worker)?;
        if let Some(fact) = sockets.get_mut(&self.id) {
            fact.endpoints
                .retain(|old| old.origin != observation.origin);
            if fact.endpoints.len() < 8 {
                fact.endpoints.push(observation);
            }
        }
        drop(sockets);
        self.discovery.remove(&transaction);
        self.changed();
        Ok(())
    }
}
