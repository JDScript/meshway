//! Roaming: this Node's own path changes and following a peer to a new
//! source once that source answers its challenge.
use super::*;

impl Owner {
    /// The underlay path changed: every channel proves itself at once, so a
    /// peer sees this Node's new source within a round trip. The path is a
    /// new one: its size is searched again and the old measurement withdrawn
    /// meanwhile, as when a peer's channel follows it ([`Self::roam_answer`]).
    pub(super) fn roamed(&mut self) {
        for channel in self.channels.values_mut() {
            channel.roam = ROAM_PROBES;
            channel.next_probe = Instant::now();
            // An inbound channel is the peer's to dial again; ending it here
            // would only stop the Pings that let the peer follow.
            if channel.fact.acquisition != LinkAcquisition::AcceptedInbound {
                channel.roam_until = Some(Instant::now() + ROAM_ANSWER);
            }
            channel.mtu = mtu::Search::new();
            if let Ok(mut known) = self.shared.path_mtu.lock() {
                known.remove(&channel.fact.id);
            }
        }
        self.measure_mtu();
    }

    /// A channel whose roaming probes went unanswered past [`ROAM_ANSWER`]
    /// ends its health window now: the peer could not follow this Node's new
    /// source, and the channel is acquired again (for a peer behind NAT, by
    /// a new coordinated traversal this Node starts).
    pub(super) fn roam_unanswered(&mut self) -> Result<(), Error> {
        let now = Instant::now();
        for channel in self.channels.values_mut() {
            if channel.roam_until.is_none_or(|until| now < until) {
                continue;
            }
            channel.roam_until = None;
            diagnostic_link(
                &self.shared,
                &channel.fact,
                &channel.scope,
                "roam",
                "unanswered",
                None,
            );
            *channel.health.lock().map_err(|_| Error::Worker)? = now;
        }
        Ok(())
    }

    /// A Ping from an unknown source that one of this socket's selected
    /// channels signed: its peer roamed. Challenge the new source with a
    /// fresh signed Ping of that channel; nothing changes until its Pong.
    /// Returns whether a channel claimed the message.
    pub(super) async fn roam_challenge(
        &mut self,
        remote: SocketAddr,
        body: &[u8],
    ) -> Result<bool, Error> {
        if self.roaming.contains_key(&remote) {
            return Ok(true);
        }
        let plan = self.shared.plan();
        let Some(current) = self
            .channels
            .iter()
            .find(|(_, channel)| {
                self.selected(channel)
                    && channel
                        .transcript
                        .verify_health(&plan, channel.fact.peer, Kind::Ping, body)
                        .is_ok()
            })
            .map(|(current, _)| *current)
        else {
            return Ok(false);
        };
        if self.roaming.len() >= MAX_ROAM_CHALLENGES
            || self
                .roaming
                .values()
                .filter(|challenge| challenge.channel == current)
                .count()
                >= MAX_ROAM_SOURCES
            || remote.is_ipv4() != self.local.is_ipv4()
        {
            return Ok(true);
        }
        let channel = &self.channels[&current];
        let scope = Scope {
            cancellation: channel.scope.cancellation.child_token(),
            deadline: Some(Instant::now() + ROAM_CHALLENGE),
            edge: channel.scope.edge.clone(),
            health: None,
        };
        // A dial or traversal socket sends only to its own recipients: the
        // challenge alone may reach this source, for its own lifetime.
        if self
            .path
            .authorize_challenge(remote, Arc::new(scope.clone()))
            .is_err()
        {
            scope.cancellation.cancel();
            return Ok(true);
        }
        let nonce = adjacency::boot_nonce(&self.shared.boot)?;
        let output = adjacency::encode(
            Kind::Ping,
            &channel
                .transcript
                .health(&self.shared.keys, &plan, Kind::Ping, &nonce)?,
        )?;
        diagnostic_link(
            &self.shared,
            &channel.fact,
            &channel.scope,
            "roam",
            "source_challenged",
            Some((remote, nonce)),
        );
        self.roaming.insert(
            remote,
            RoamChallenge {
                channel: current,
                nonce,
                output,
                scope,
                retry: Retry::due(),
            },
        );
        self.roam_tick().await;
        Ok(true)
    }

    /// Send due challenges; drop ended ones.
    pub(super) async fn roam_tick(&mut self) {
        self.roaming.retain(|_, challenge| {
            let live = challenge.scope.check().is_ok();
            if !live {
                challenge.scope.cancellation.cancel();
            }
            live
        });
        for (remote, challenge) in &mut self.roaming {
            if challenge.retry.take(RETRY, ROAM_PROBES) {
                let _ = send_control(
                    &self.sender,
                    &challenge.scope,
                    *remote,
                    challenge.output.clone(),
                )
                .await;
            }
        }
    }

    /// A Pong from an unknown source: if it answers that source's challenge,
    /// the channel moves there. Its identity, transcript and boot stay; only
    /// the recipient changes, and the path size is searched again.
    pub(super) fn roam_answer(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        let Some(challenge) = self.roaming.get(&remote) else {
            return Ok(());
        };
        let Some(channel) = self.channels.get(&challenge.channel) else {
            return Ok(());
        };
        let plan = self.shared.plan();
        if challenge.scope.check().is_err()
            || channel
                .transcript
                .verify_health(&plan, channel.fact.peer, Kind::Pong, body)
                .ok()
                != Some(challenge.nonce)
        {
            return Ok(());
        }
        let challenge = self.roaming.remove(&remote).ok_or(Error::Worker)?;
        challenge.scope.cancellation.cancel();
        let old = challenge.channel;
        self.roaming.retain(|_, other| {
            let keep = other.channel != old;
            if !keep {
                other.scope.cancellation.cancel();
            }
            keep
        });
        if self.channels.contains_key(&remote) {
            return Ok(());
        }
        let mut channel = self.channels.remove(&old).ok_or(Error::Worker)?;
        // One grant per channel, for its current remote: the next move
        // revokes it, so a channel follows any number of moves.
        let grant = Scope {
            cancellation: channel.scope.cancellation.child_token(),
            ..channel.scope.clone()
        };
        let moved = self
            .path
            .authorize_source(remote, Arc::new(grant.clone()))
            .is_ok()
            && channel.scope.check().is_ok()
            && {
                let mut links = self.shared.links.lock().map_err(|_| Error::Worker)?;
                match links.get_mut(&channel.fact.peer) {
                    Some(link) if link.fact.id == channel.fact.id => {
                        link.fact.remote = remote;
                        true
                    }
                    _ => false,
                }
            };
        if !moved {
            grant.cancellation.cancel();
            self.channels.insert(old, channel);
            return Ok(());
        }
        if let Some(previous) = channel.source_grant.replace(grant.cancellation) {
            previous.cancel();
        }
        channel.fact.remote = remote;
        channel.mtu = mtu::Search::new();
        channel.probe = None;
        channel.roam = 0;
        channel.roam_until = None;
        channel.confirmed = true;
        channel.next_probe = Instant::now() + PROBE_INTERVAL;
        *channel.health.lock().map_err(|_| Error::Worker)? = Instant::now() + HEALTH_WINDOW;
        if let Ok(mut known) = self.shared.path_mtu.lock() {
            known.remove(&channel.fact.id);
        }
        diagnostic_link(
            &self.shared,
            &channel.fact,
            &channel.scope,
            "roam",
            "source_moved",
            Some((old, channel.fact.id)),
        );
        self.channels.insert(remote, channel);
        self.measure_mtu();
        self.changed();
        Ok(())
    }
}
