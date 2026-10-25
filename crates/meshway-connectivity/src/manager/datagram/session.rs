//! Channel establishment: a listener's INIT, source challenge and ACK, the
//! dialer's handshake messages, passive restart recovery, and promotion of a
//! verified transcript to a selected channel.
use super::*;

impl Owner {
    #[allow(clippy::too_many_arguments)]
    fn promote(
        &mut self,
        plan: &Arc<Plan>,
        peer: NodeId,
        carrier: Carrier,
        remote: SocketAddr,
        transcript: Transcript,
        capacity: OwnedSemaphorePermit,
        ready_until: Instant,
        supersedes: Replacement,
        acquisition: LinkAcquisition,
    ) -> Result<bool, Error> {
        self.promote_scoped(
            plan,
            peer,
            carrier,
            remote,
            transcript,
            capacity,
            ready_until,
            supersedes,
            acquisition,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn promote_scoped(
        &mut self,
        plan: &Arc<Plan>,
        peer: NodeId,
        carrier: Carrier,
        remote: SocketAddr,
        transcript: Transcript,
        capacity: OwnedSemaphorePermit,
        ready_until: Instant,
        supersedes: Replacement,
        acquisition: LinkAcquisition,
        attempt: Option<meshway_control::connectivity::AttemptScope>,
    ) -> Result<bool, Error> {
        if self
            .channels
            .get(&remote)
            .is_some_and(|channel| channel.fact.peer != peer)
        {
            return Ok(false);
        }
        let health = Arc::new(Mutex::new(Instant::now() + HEALTH_WINDOW));
        let mut scope = self.shared.scope(Some((peer, carrier.clone())), false);
        scope.health = Some(health.clone());
        if scope.check().is_err() {
            return Ok(false);
        }
        if let Some(attempt) = &attempt {
            attempt.ensure_current().map_err(|_| Error::Authority)?;
            self.authorization
                .as_ref()
                .ok_or(Error::Underlay)?
                .authorize_channel(attempt, Arc::new(scope.clone()), remote)?;
        }
        let fact = LinkFact {
            id: transcript.id(),
            socket_id: self.id,
            peer,
            peer_boot: transcript.peer_boot(carrier.dialer == plan.node),
            carrier,
            local: self.local,
            remote,
            acquisition,
        };
        let selected = install_ready(
            &self.shared,
            ReadyPeer {
                fact: fact.clone(),
                sender: self.sender.clone(),
                scope: scope.clone(),
                plan: plan.clone(),
                supersedes,
                attempt,
            },
        );
        match selected {
            Ok(true) => {}
            Ok(false) => {
                scope.cancellation.cancel();
                return Ok(false);
            }
            Err(error) => {
                scope.cancellation.cancel();
                return Err(error);
            }
        }
        if let Some(old) = self.channels.remove(&remote) {
            old.scope.cancellation.cancel();
        }
        self.channels.insert(
            remote,
            Channel {
                fact,
                transcript,
                scope,
                health,
                probe: None,
                next_probe: Instant::now() + PROBE_INTERVAL,
                last_response: Instant::now() - RETRY,
                last_probe_response: Instant::now() - RETRY,
                confirmed: false,
                ready_until,
                budget: ControlBudget::with_limit(16),
                _capacity: capacity,
                recovery: None,
                mtu: mtu::Search::new(),
                roam: 0,
                roam_until: None,
                unanswered: 0,
                peer_pings: 0,
                answered_at: Instant::now(),
                source_grant: None,
            },
        );
        self.measure_mtu();
        self.changed();
        Ok(true)
    }

    async fn repeat_intro(&mut self, remote: SocketAddr, kind: Kind, body: &[u8]) -> bool {
        if let Some(attempt) = self.pending.get_mut(&remote) {
            if attempt.scope.check().is_ok()
                && attempt.intro_kind == kind
                && attempt.intro == body
                && attempt.responses.take(RETRY, MAX_SENDS)
            {
                let _ = send_control(
                    &self.sender,
                    &attempt.scope,
                    remote,
                    attempt.challenge.clone(),
                )
                .await;
            }
            return true;
        }
        false
    }

    fn incoming_carriers(&self) -> Result<Vec<(NodeId, Carrier)>, Error> {
        let Some(listener) = &self.listener else {
            return Ok(Vec::new());
        };
        Ok(self
            .shared
            .plan()
            .incoming(&listener.key, TransportProtocol::NativeUdp))
    }

    pub(super) async fn init(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        if self.listener.is_none() || self.repeat_intro(remote, Kind::Init, body).await {
            return Ok(());
        }
        // A fresh source challenge may coexist with the old channel even on
        // the same tuple. Nothing in the channel map changes until valid ACK.
        if self.pending.len() + self.channels.len() >= self.shared.limits.connections {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        // Only the claimed dialer's carrier is checked; its exact context and
        // signature are still verified below.
        let listener = self.listener.as_ref().ok_or(Error::Worker)?;
        let carriers: Vec<_> = adjacency::claimed_dialer(body)
            .and_then(|peer| {
                cfg.accepting(peer, &listener.key, TransportProtocol::NativeUdp)
                    .map(|carrier| (peer, carrier))
            })
            .into_iter()
            .collect();
        let Ok((peer, carrier, initiation)) = adjacency::incoming(cfg, &carriers, body) else {
            self.unverified_init_rejections = self.unverified_init_rejections.saturating_add(1);
            if self.unverified_init_rejections.is_power_of_two() && diagnostic_event() {
                tracing::debug!(operation = "init_rejected", cause = "signature_or_context",
                    remote = %remote, count = self.unverified_init_rejections,
                    "Connectivity unauthenticated initiation rejected");
            }
            return Ok(());
        };
        if self
            .channels
            .get(&remote)
            .is_some_and(|channel| channel.fact.peer != peer)
        {
            return Ok(());
        }
        if self.replay.contains(&(peer, initiation.client))
            || self.pending.values().any(|attempt| attempt.peer == peer)
        {
            return Ok(());
        }
        let supersedes = {
            let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
            match links.get(&peer).filter(|link| link.scope.check().is_ok()) {
                Some(link) if link.fact.carrier.protocol == TransportProtocol::NativeUdp => {
                    diagnostic_link(
                        &self.shared,
                        &link.fact,
                        &link.scope,
                        "signed_init_eligible",
                        "udp_predecessor_captured",
                        Some((remote, initiation.client)),
                    );
                    Replacement::Channel(link.fact.id)
                }
                // A healthy stream: the dialer returns to the edge's earlier
                // Native UDP carrier. It acknowledges only an attempt made to
                // replace exactly this stream, so its ACK decides.
                Some(link) => {
                    diagnostic_link(
                        &self.shared,
                        &link.fact,
                        &link.scope,
                        "signed_init_eligible",
                        "stream_predecessor_captured",
                        Some((remote, initiation.client)),
                    );
                    Replacement::Channel(link.fact.id)
                }
                None => Replacement::Ranked,
            }
        };
        self.challenge_source(
            peer,
            carrier,
            initiation,
            remote,
            Kind::Init,
            body,
            supersedes,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn challenge_source(
        &mut self,
        peer: NodeId,
        carrier: Carrier,
        initiation: Initiation,
        remote: SocketAddr,
        intro_kind: Kind,
        body: &[u8],
        supersedes: Replacement,
    ) -> Result<(), Error> {
        let Ok(capacity) = self.shared.capacity.clone().try_acquire_owned() else {
            return Ok(());
        };
        let scope = self.shared.scope(Some((peer, carrier.clone())), true);
        if scope.check().is_err() {
            return Ok(());
        }
        let rebuild = self.replay.len() >= MAX_REPLAY;
        if !rebuild {
            self.replay.insert((peer, initiation.client));
        }
        // At capacity no historical nonce is forgotten and no extra history is
        // allocated. Only this bounded pending slot can admit its fresh ACK.
        // Expiring/retrying the slot generates another random server nonce, so
        // its old ACK is unusable. A copied INIT alone cannot force rebuilding:
        // the current source must finish this challenge and win normal CAS.
        let transcript = initiation.respond(&self.shared.boot)?;
        let challenge = adjacency::encode(
            Kind::Challenge,
            &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Challenge)?,
        )?;
        // One input produces at most one smaller response, with no autonomous
        // server retry timer before the actual source has completed a challenge.
        if challenge.len() > body.len() + 11 {
            return Ok(());
        }
        let attempt = ServerAttempt {
            peer,
            carrier,
            transcript,
            intro: body.to_vec(),
            intro_kind,
            challenge,
            scope,
            responses: Retry::sent(RETRY),
            capacity,
            supersedes,
            rebuild,
        };
        let _ = send_control(
            &self.sender,
            &attempt.scope,
            remote,
            attempt.challenge.clone(),
        )
        .await;
        self.pending.insert(remote, attempt);
        Ok(())
    }

    pub(super) async fn ack(&mut self, remote: SocketAddr, body: &[u8]) -> Result<(), Error> {
        let plan = self.shared.plan();
        let cfg = &*plan;
        // Match the pending fresh transcript first. An old channel on the same
        // tuple must neither swallow a fresh ACK nor authorize it by address.
        let pending_matches = self.pending.get(&remote).is_some_and(|attempt| {
            attempt.scope.check().is_ok()
                && attempt
                    .transcript
                    .verify(cfg, attempt.peer, Kind::Ack, body)
                    .is_ok()
        });
        if !pending_matches {
            if let Some(channel) = self.channels.get_mut(&remote)
                && self.listener.is_some()
                && !channel.confirmed
                && Instant::now() < channel.ready_until
                && channel.scope.check().is_ok()
                && Instant::now() >= channel.last_response + RETRY
                && channel
                    .transcript
                    .verify(cfg, channel.fact.peer, Kind::Ack, body)
                    .is_ok()
            {
                channel.last_response = Instant::now();
                let bytes = adjacency::encode(
                    Kind::Ready,
                    &channel.transcript.message(
                        &self.shared.keys,
                        &self.shared.plan(),
                        Kind::Ready,
                    )?,
                )?;
                let _ = send_control(&self.sender, &channel.scope, remote, bytes).await;
            }
            return Ok(());
        }
        let attempt = self.pending.get(&remote).ok_or(Error::Worker)?;
        let bytes = adjacency::encode(
            Kind::Ready,
            &attempt
                .transcript
                .message(&self.shared.keys, &self.shared.plan(), Kind::Ready)?,
        )?;
        let attempt = self.pending.remove(&remote).ok_or(Error::Worker)?;
        let scope = attempt.scope;
        if self.promote(
            &plan,
            attempt.peer,
            attempt.carrier,
            remote,
            attempt.transcript,
            attempt.capacity,
            scope.deadline_or(self.shared.limits.handshake_timeout),
            attempt.supersedes,
            LinkAcquisition::AcceptedInbound,
        )? {
            if attempt.rebuild {
                // Authentication and normal selection succeeded. End this
                // listener's generation; the Manager rebinds the listener
                // alone. Never leave a live listener permanently unable to
                // accept a new channel, or clear its replay/ABA state
                // underneath other pending handshakes.
                tracing::warn!("UDP replay capacity exhausted; listener rebuild required");
                self.shared.stop.cancel();
                return Err(Error::Capacity);
            }
            // READY attests completed authentication on the selected owner, not
            // merely a valid ACK for a candidate rejected by selection.
            let _ = send_control(&self.sender, &scope, remote, bytes).await;
        }
        Ok(())
    }

    pub(super) async fn client_message(
        &mut self,
        remote: SocketAddr,
        kind: Kind,
        body: &[u8],
    ) -> Result<(), Error> {
        if self
            .outbound
            .as_ref()
            .is_some_and(|outbound| outbound.established)
        {
            return self.recovery_message(remote, kind, body).await;
        }
        let Some(outbound) = &mut self.outbound else {
            return Ok(());
        };
        if outbound.established || outbound.scope.check().is_err() {
            return Ok(());
        }
        let Some(attempt) = outbound.attempts.get_mut(&remote) else {
            return Ok(());
        };
        let plan = self.shared.plan();
        let cfg = &*plan;
        if kind == Kind::Challenge {
            let Ok(transcript) = attempt.initiation.challenge(cfg, outbound.peer, body) else {
                return Ok(());
            };
            let candidate = LinkFact {
                id: transcript.id(),
                socket_id: self.id,
                peer: outbound.peer,
                peer_boot: transcript.peer_boot(true),
                carrier: outbound.carrier.clone(),
                local: self.local,
                remote,
                acquisition: outbound.acquisition.clone(),
            };
            if self
                .shared
                .links
                .lock()
                .map_err(|_| Error::Worker)?
                .get(&outbound.peer)
                .is_some_and(|link| {
                    link.scope.check().is_ok()
                        && Some(link.fact.id) != outbound.supersedes
                        && preferred_existing(&link.fact, &candidate)
                })
            {
                // Do not sign a superseding ACK for a stale initial candidate
                // already beaten by this initiator's healthy ranked selection.
                return Err(Error::Unavailable);
            }
            match &attempt.phase {
                ClientPhase::Acked {
                    transcript: old, ..
                } => {
                    if old.id() != transcript.id() {
                        return Ok(());
                    }
                }
                ClientPhase::Init { .. } => {
                    let output = adjacency::encode(
                        Kind::Ack,
                        &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Ack)?,
                    )?;
                    attempt.phase = ClientPhase::Acked { transcript, output };
                    attempt.retry = Retry::due();
                }
            }
            // Retransmission is timer-paced, including duplicate challenges.
            return Ok(());
        }
        let ClientPhase::Acked { transcript, .. } = &attempt.phase else {
            return Ok(());
        };
        if transcript
            .verify(cfg, outbound.peer, Kind::Ready, body)
            .is_err()
        {
            return Ok(());
        }
        let Some(ClientAttempt {
            phase: ClientPhase::Acked { transcript, .. },
            ..
        }) = outbound.attempts.remove(&remote)
        else {
            return Err(Error::Worker);
        };
        let peer = outbound.peer;
        let carrier = outbound.carrier.clone();
        let capacity = outbound.capacity.take().ok_or(Error::Worker)?;
        outbound.established = true;
        outbound.attempts.clear();
        let deadline = outbound
            .scope
            .deadline_or(self.shared.limits.handshake_timeout);
        let acquisition = outbound.acquisition.clone();
        let supersedes = outbound
            .supersedes
            .map_or(Replacement::Ranked, Replacement::Channel);
        if !self.promote(
            &plan,
            peer,
            carrier,
            remote,
            transcript,
            capacity,
            deadline,
            supersedes,
            acquisition,
        )? {
            return Err(Error::Unavailable);
        }
        Ok(())
    }

    /// A passive restart cannot dial a dial-only peer. Verify that peer's signed
    /// outstanding Ping, then require a NEW source challenge/ACK. Neither an
    /// unknown datagram nor a copied old probe retires any channel.
    pub(super) async fn probe_recovery(
        &mut self,
        remote: SocketAddr,
        body: &[u8],
    ) -> Result<(), Error> {
        if self.listener.is_none()
            || self.repeat_intro(remote, Kind::Ping, body).await
            || self.pending.len() + self.channels.len() >= self.shared.limits.connections
        {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        for (peer, carrier) in self.incoming_carriers()? {
            let Ok((old_id, initiation)) = Initiation::from_probe(cfg, peer, &carrier, body) else {
                continue;
            };
            if self.replay.contains(&(peer, initiation.client))
                || self.pending.values().any(|attempt| attempt.peer == peer)
            {
                return Ok(());
            }
            let supersedes = {
                let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
                match links.get(&peer) {
                    None => Replacement::Vacant(links.last.get(&peer).copied()),
                    Some(link)
                        if link.fact.id == old_id
                            && link.fact.carrier == carrier
                            && link.scope.check().is_ok() =>
                    {
                        Replacement::Channel(old_id)
                    }
                    Some(_) => return Ok(()),
                }
            };
            if diagnostic_event() {
                tracing::debug!(operation = "probe_recovery", peer = %peer, channel = ?old_id, remote = %remote,
                    "Connectivity signed probe challenged");
            }
            return self
                .challenge_source(
                    peer,
                    carrier,
                    initiation,
                    remote,
                    Kind::Ping,
                    body,
                    supersedes,
                )
                .await;
        }
        Ok(())
    }

    async fn recovery_message(
        &mut self,
        remote: SocketAddr,
        kind: Kind,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(channel) = self.channels.get(&remote) else {
            return Ok(());
        };
        if !self.selected(channel) {
            return Ok(());
        }
        let plan = self.shared.plan();
        let cfg = &*plan;
        if channel.fact.carrier.dialer != cfg.node {
            return Ok(());
        }
        if kind == Kind::Challenge {
            if channel.recovery.is_some() {
                return Ok(());
            }
            let Some(probe) = channel.probe else {
                return Ok(());
            };
            let Ok(transcript) =
                channel
                    .transcript
                    .probe_challenge(cfg, channel.fact.peer, probe, body)
            else {
                return Ok(());
            };
            let Ok(capacity) = self.shared.capacity.clone().try_acquire_owned() else {
                return Ok(());
            };
            let mut scope = self.shared.scope(
                Some((channel.fact.peer, channel.fact.carrier.clone())),
                true,
            );
            // Pending ACK output also dies with the captured predecessor. The
            // replacement receives its own scope only after verified READY/CAS.
            scope.cancellation = channel.scope.cancellation.child_token();
            scope.health = Some(channel.health.clone());
            scope.check()?;
            let output = adjacency::encode(
                Kind::Ack,
                &transcript.message(&self.shared.keys, &self.shared.plan(), Kind::Ack)?,
            )?;
            self.channels
                .get_mut(&remote)
                .ok_or(Error::Worker)?
                .recovery = Some(Recovery {
                transcript,
                scope,
                output,
                retry: Retry::due(),
                capacity,
            });
            return Ok(());
        }
        let Some(recovery) = &channel.recovery else {
            return Ok(());
        };
        if kind != Kind::Ready
            || recovery.scope.check().is_err()
            || recovery
                .transcript
                .verify(cfg, channel.fact.peer, Kind::Ready, body)
                .is_err()
        {
            return Ok(());
        }
        let peer = channel.fact.peer;
        let carrier = channel.fact.carrier.clone();
        let old_id = channel.fact.id;
        let acquisition = channel.fact.acquisition.clone();
        let recovery = self
            .channels
            .get_mut(&remote)
            .ok_or(Error::Worker)?
            .recovery
            .take()
            .ok_or(Error::Worker)?;
        let _ = self.promote(
            &plan,
            peer,
            carrier,
            remote,
            recovery.transcript,
            recovery.capacity,
            recovery
                .scope
                .deadline_or(self.shared.limits.handshake_timeout),
            Replacement::Channel(old_id),
            acquisition,
        )?;
        recovery.scope.cancellation.cancel();
        Ok(())
    }
}
