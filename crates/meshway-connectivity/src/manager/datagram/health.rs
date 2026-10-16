//! The owner tick (health Pings, size probes, retransmission) and answers
//! to a peer's health and size probes.
use super::*;

impl Owner {
    pub(super) async fn tick(&mut self) -> Result<(), Error> {
        if self
            .channels
            .values()
            .any(|channel| Instant::now() >= channel.next_probe)
        {
            self.measure_mtu();
        }
        self.stun_tick().await?;
        self.traversal_tick().await?;
        self.roam_tick().await;
        self.lifetime.check()?;
        self.pending
            .retain(|_, attempt| attempt.scope.check().is_ok());
        self.roam_unanswered()?;
        let stale: Vec<_> = self
            .channels
            .iter()
            .filter(|(_, channel)| !self.selected(channel))
            .map(|(remote, _)| *remote)
            .collect();
        for remote in stale {
            self.lifetime.check()?;
            if let Some(channel) = self.channels.remove(&remote) {
                diagnostic_link(
                    &self.shared,
                    &channel.fact,
                    &channel.scope,
                    "channel_cleanup",
                    "scope_invalid_or_deselected",
                    None,
                );
                channel.health_ended();
                channel.scope.cancellation.cancel();
                retire(&self.shared, &channel.fact)?;
                self.changed();
            }
        }
        if let Some(outbound) = &mut self.outbound {
            if outbound.established {
                if self.channels.is_empty() {
                    return Err(Error::Unavailable);
                }
            } else {
                outbound.scope.check()?;
                if self
                    .shared
                    .links
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .get(&outbound.peer)
                    .is_some_and(|link| {
                        link.scope.check().is_ok()
                            && Some(link.fact.id) != outbound.supersedes
                            && (link.fact.carrier == outbound.carrier
                                || link.fact.carrier.protocol != outbound.carrier.protocol)
                    })
                {
                    return Err(Error::Unavailable);
                }
                for (remote, attempt) in &mut outbound.attempts {
                    if !attempt.retry.take(RETRY, MAX_SENDS) {
                        continue;
                    }
                    let _ = send_control(
                        &self.sender,
                        &outbound.scope,
                        *remote,
                        attempt.output().to_vec(),
                    )
                    .await;
                }
            }
        }
        for (remote, channel) in &mut self.channels {
            if channel
                .recovery
                .as_ref()
                .is_some_and(|recovery| recovery.scope.check().is_err())
                && let Some(recovery) = channel.recovery.take()
            {
                recovery.scope.cancellation.cancel();
            }
            if let Some(recovery) = &mut channel.recovery
                && recovery.retry.take(RETRY, MAX_SENDS)
            {
                let _ = send_control(
                    &self.sender,
                    &recovery.scope,
                    *remote,
                    recovery.output.clone(),
                )
                .await;
            }
            if channel.scope.check().is_ok()
                && let Some(size) = channel
                    .mtu
                    .due(probe_route(self.path.as_ref(), *remote), Instant::now())
            {
                // A DF size probe signed for its exact length. One that cannot
                // be built ends this measurement only, never the owner.
                let nonce = adjacency::boot_nonce(&self.shared.boot)?;
                match channel.transcript.probe(
                    &self.shared.keys,
                    &self.shared.plan(),
                    &nonce,
                    usize::from(size.saturating_sub(header(*remote))),
                ) {
                    Ok(bytes) => {
                        channel.mtu.sent(nonce);
                        let _ = send_control(&self.sender, &channel.scope, *remote, bytes).await;
                    }
                    Err(error) => {
                        tracing::debug!(?error, peer = %channel.fact.peer, channel = ?channel.fact.id,
                            size, "UDP size probe not built; measurement ended");
                        channel.mtu.abandon(Instant::now());
                    }
                }
            }
            if channel.scope.check().is_err() || Instant::now() < channel.next_probe {
                continue;
            }
            // Only a response to this node's fresh challenge renews health.
            // Received business packets and remote Ping messages do not.
            let nonce = match channel.probe {
                Some(nonce) => nonce,
                None => adjacency::boot_nonce(&self.shared.boot)?,
            };
            channel.probe = Some(nonce);
            channel.next_probe = Instant::now()
                + if channel.roam > 0 {
                    channel.roam -= 1;
                    RETRY
                } else {
                    PROBE_INTERVAL
                };
            let bytes = adjacency::encode(
                Kind::Ping,
                &channel.transcript.health(
                    &self.shared.keys,
                    &self.shared.plan(),
                    Kind::Ping,
                    &nonce,
                )?,
            )?;
            channel.unanswered = channel.unanswered.saturating_add(1);
            if let Err(error) = send_control(&self.sender, &channel.scope, *remote, bytes).await {
                tracing::debug!(?error, peer = %channel.fact.peer, channel = ?channel.fact.id,
                    "UDP health probe send failed");
                meshway_logging::profile::event("manager.udp.health", "send_failed");
            }
        }
        Ok(())
    }

    pub(super) async fn health(
        &mut self,
        remote: SocketAddr,
        kind: Kind,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(channel) = self.channels.get_mut(&remote) else {
            // A known channel's peer at a new source roamed; without one
            // (a restarted listener) the source is challenged as recovery.
            return match kind {
                Kind::Ping if self.roam_challenge(remote, body).await? => Ok(()),
                Kind::Ping => self.probe_recovery(remote, body).await,
                Kind::Pong => self.roam_answer(remote, body),
                _ => Ok(()),
            };
        };
        if channel.scope.check().is_err() {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        let Ok(nonce) = channel
            .transcript
            .verify_health(cfg, channel.fact.peer, kind, body)
        else {
            return Ok(());
        };
        if kind == Kind::Pong {
            if channel.probe != Some(nonce) {
                if channel.mtu.answered(&nonce, Instant::now()) {
                    self.measure_mtu();
                }
                return Ok(());
            }
            if channel.scope.check().is_err() {
                return Ok(());
            }
            channel.probe = None;
            channel.confirmed = true;
            channel.roam = 0;
            channel.roam_until = None;
            channel.unanswered = 0;
            channel.peer_pings = 0;
            channel.answered_at = Instant::now();
            *channel.health.lock().map_err(|_| Error::Worker)? = Instant::now() + HEALTH_WINDOW;
        } else {
            channel.peer_pings = channel.peer_pings.saturating_add(1);
            if Instant::now() < channel.last_response + RETRY {
                return Ok(());
            }
            channel.last_response = Instant::now();
            let bytes = adjacency::encode(
                Kind::Pong,
                &channel.transcript.health(
                    &self.shared.keys,
                    &self.shared.plan(),
                    Kind::Pong,
                    &nonce,
                )?,
            )?;
            let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
        }
        Ok(())
    }

    /// Answer a peer's size probe with a Pong over its nonce, only if it was
    /// signed for the length actually received.
    pub(super) async fn answer_size_probe(
        &mut self,
        remote: SocketAddr,
        datagram: usize,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(channel) = self.channels.get_mut(&remote) else {
            return Ok(());
        };
        if channel.scope.check().is_err() || Instant::now() < channel.last_probe_response + RETRY {
            return Ok(());
        }
        let plan = self.shared.plan();
        let Ok(nonce) = channel
            .transcript
            .verify_probe(&plan, channel.fact.peer, datagram, body)
        else {
            return Ok(());
        };
        channel.last_probe_response = Instant::now();
        let bytes = adjacency::encode(
            Kind::Pong,
            &channel
                .transcript
                .health(&self.shared.keys, &plan, Kind::Pong, &nonce)?,
        )?;
        let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
        Ok(())
    }
}
