//! Offer-bound acquisition inside the existing endpoint owner. The envelope is
//! only demultiplexing; every handshake body uses the canonical signed exchange.
use super::*;
use meshway_control::connectivity::{AttemptScope, ValidatedOffer};
use meshway_transport::SendLifetime;

pub(super) const MAGIC: &[u8; 8] = b"MWTRV1\0\0";
const HEADER: usize = 41;

pub(super) struct Attempt {
    offer: ValidatedOffer,
    active: Option<AttemptScope>,
    capacity: Option<OwnedSemaphorePermit>,
    initiation: Option<Initiation>,
    transcript: Option<Transcript>,
    output: Vec<u8>,
    /// Per-output pacing; `total` bounds every output of the attempt.
    retry: Retry,
    total: u8,
    replacement: Replacement,
    completed: bool,
}
impl Attempt {
    pub fn cancel(&self) {
        self.offer.cancel();
        if let Some(active) = &self.active {
            active.cancel();
        }
    }
    fn output(&mut self, kind: u8, body: Vec<u8>) {
        self.output = envelope(&self.offer, kind, &body);
        self.retry = Retry::due();
    }
}

fn envelope(offer: &ValidatedOffer, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + body.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(offer.offer().digest.as_bytes());
    bytes.push(kind);
    bytes.extend_from_slice(body);
    bytes
}

struct Lifetime {
    scope: Scope,
    active: AttemptScope,
    original: ValidatedOffer,
}
impl std::fmt::Debug for Lifetime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraversalOutput").finish_non_exhaustive()
    }
}
impl SendLifetime for Lifetime {
    fn is_current(&self) -> bool {
        self.scope.check().is_ok()
            && self.active.ensure_current().is_ok()
            && self.original.ensure_current().is_ok()
    }
    fn invalidated(
        self: Arc<Self>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            tokio::select! {
                _ = self.active.invalidated() => {},
                _ = self.original.invalidated() => {},
                _ = self.scope.run(std::future::pending::<Result<(), Error>>()) => {},
            }
        })
    }
}

pub(super) async fn send(
    sender: &AsyncMutex<Sender>,
    lifetime: Arc<dyn SendLifetime>,
    deadline: Instant,
    remote: SocketAddr,
    bytes: Vec<u8>,
) -> Result<(), Error> {
    let cancellation = CancellationToken::new();
    let _cancel = cancellation.clone().drop_guard();
    let deadline = deadline.min(Instant::now() + OUTPUT_WAIT);
    let context = SendContext {
        cancellation,
        deadline: Some(deadline),
        lifetime: Some(lifetime.clone()),
    };
    let mut frames = VecDeque::from([Frame::new(bytes, Some(remote))]);
    tokio::select! { biased;
        _ = lifetime.clone().invalidated() => Err(Error::Authority),
        result = tokio::time::timeout_at(deadline, async {
            if !lifetime.is_current() { return Err(Error::Authority); }
            let admitted = sender.lock().await.try_send(&mut frames, context);
            if admitted.accepted != 1 { return Err(Error::Capacity); }
            let completion = admitted.completion.ok_or(Error::Worker)?.wait().await.map_err(|_| Error::Worker)?;
            if completion.frames.len() != 1 || !matches!(completion.frames[0].outcome, SendOutcome::LocalComplete) { return Err(Error::Unavailable); }
            if !lifetime.is_current() { return Err(Error::Authority); }
            Ok(())
        }) => result.map_err(|_| Error::Cancelled)?,
    }
}

impl Owner {
    pub(super) fn traversal_command(
        &mut self,
        command: super::super::traversal::Command,
    ) -> Result<(), Error> {
        use super::super::traversal::Command;
        match command {
            Command::Discover {
                context,
                server,
                targets,
            } => self.discover(context, server, targets),
            Command::Reserve { offer, reply } => {
                if reply.is_closed() {
                    return Ok(());
                }
                let id = offer.offer().attempt_id;
                let result = self.reserve_traversal(offer);
                if reply.send(result).is_err()
                    && let Some(attempt) = self.traversal.remove(&id)
                {
                    attempt.cancel();
                }
                Ok(())
            }
            Command::Activate(active) => {
                if super::super::traversal::matching_endpoint(
                    &self.shared,
                    active.validated_offer(),
                ) != Some(self.id)
                {
                    active.cancel();
                    return Ok(());
                }
                let id = active.validated_offer().offer().attempt_id;
                let Some(attempt) = self.traversal.get_mut(&id) else {
                    active.cancel();
                    return Ok(());
                };
                if attempt.active.is_some() {
                    return Ok(());
                }
                if active.ensure_current().is_err()
                    || attempt.offer.ensure_current().is_err()
                    || active.validated_offer().offer() != attempt.offer.offer()
                {
                    active.cancel();
                    attempt.cancel();
                    return Ok(());
                }
                let Some(authorization) = &self.authorization else {
                    active.cancel();
                    return Ok(());
                };
                if authorization.authorize_attempt(&active).is_err() {
                    active.cancel();
                    attempt.cancel();
                    return Ok(());
                }
                let (initiation, intro) = Initiation::new_bound(
                    &self.shared.identity,
                    &self.shared.plan,
                    &attempt.offer.offer().carrier,
                    Some(attempt.offer.offer().digest),
                )?;
                let dialer = attempt.offer.offer().carrier.dialer
                    == attempt.offer.local().candidate.socket.node_id;
                attempt.initiation = Some(initiation);
                attempt.active = Some(active);
                // A signed punch from the responder opens its NAT filter before
                // it receives INIT. It grants no link or business authority.
                attempt.output(if dialer { 1 } else { 0 }, intro);
                Ok(())
            }
            Command::Cancel(id) => {
                if let Some(attempt) = self.traversal.remove(&id) {
                    attempt.cancel();
                }
                Ok(())
            }
        }
    }

    fn reserve_traversal(&mut self, offer: ValidatedOffer) -> Result<(), Error> {
        offer.ensure_current().map_err(|_| Error::Authority)?;
        if super::super::traversal::matching_endpoint(&self.shared, &offer) != Some(self.id) {
            return Err(Error::Authority);
        }
        let id = offer.offer().attempt_id;
        if let Some(previous) = self.traversal.get(&id) {
            return if previous.offer.offer() == offer.offer()
                && previous.offer.ensure_current().is_ok()
            {
                Ok(())
            } else {
                Err(Error::Authority)
            };
        }
        let peer = offer.peer().candidate.socket.node_id;
        if self.traversal.len() >= 8
            || self.traversal.values().any(|attempt| {
                !attempt.completed && attempt.offer.peer().candidate.socket.node_id == peer
            })
        {
            return Err(Error::Capacity);
        }
        let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
        let replacement = match links.get(&peer) {
            Some(link)
                if link.fact.carrier == offer.offer().carrier
                    && link.fact.carrier.protocol == TransportProtocol::NativeUdp =>
            {
                Replacement::Channel(link.fact.id)
            }
            Some(_) => return Err(Error::Unavailable),
            None => Replacement::Vacant(links.last.get(&peer).copied()),
        };
        drop(links);
        let capacity = self
            .shared
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        self.traversal.insert(
            id,
            Attempt {
                offer,
                active: None,
                capacity: Some(capacity),
                initiation: None,
                transcript: None,
                output: Vec::new(),
                retry: Retry::due(),
                total: 0,
                replacement,
                completed: false,
            },
        );
        Ok(())
    }

    pub(super) async fn traversal_tick(&mut self) -> Result<(), Error> {
        self.traversal.retain(|_, attempt| {
            let candidate_current = attempt.completed
                || self.shared.sockets.lock().is_ok_and(|sockets| {
                    sockets.get(&self.id).is_some_and(|fact| {
                        fact.endpoints.iter().any(|observation| {
                            fact.candidate(observation)
                                .is_ok_and(|candidate| candidate == attempt.offer.local().candidate)
                        })
                    })
                });
            let keep = candidate_current
                && attempt.offer.ensure_current().is_ok()
                && attempt
                    .active
                    .as_ref()
                    .is_none_or(|active| active.ensure_current().is_ok());
            if !keep {
                attempt.cancel();
            }
            keep
        });
        for attempt in self.traversal.values_mut() {
            let Some(active) = &attempt.active else {
                continue;
            };
            if attempt.output.is_empty() || attempt.total >= 32 || !attempt.retry.take(RETRY, 8) {
                continue;
            }
            attempt.total += 1;
            let remote =
                meshway_control::connectivity::udp_recipient(&attempt.offer.peer().address)
                    .map_err(|_| Error::Configuration)?;
            let lifetime = Arc::new(Lifetime {
                scope: self.lifetime.clone(),
                active: active.clone(),
                original: attempt.offer.clone(),
            });
            let _ = send(
                &self.sender,
                lifetime,
                attempt.offer.expires_at().into(),
                remote,
                attempt.output.clone(),
            )
            .await;
        }
        Ok(())
    }

    pub(super) async fn traversal_receive(
        &mut self,
        remote: SocketAddr,
        bytes: &[u8],
    ) -> Result<(), Error> {
        if bytes.len() <= HEADER
            || bytes.len() > adjacency::MAX_MESSAGE
            || !self
                .shared
                .control_budget
                .lock()
                .map_err(|_| Error::Worker)?
                .take()
        {
            return Ok(());
        }
        let Some(id) = self.traversal.iter().find_map(|(id, attempt)| {
            (attempt.offer.offer().digest.as_bytes().as_slice() == &bytes[8..40]
                && meshway_control::connectivity::udp_recipient(&attempt.offer.peer().address).ok()
                    == Some(remote))
            .then_some(*id)
        }) else {
            return Ok(());
        };
        let mut attempt = self.traversal.remove(&id).ok_or(Error::Worker)?;
        let result = self.traversal_message(&mut attempt, remote, bytes[40], &bytes[HEADER..]);
        // Bad packets never consume a reservation or stop its valid retries.
        if attempt.offer.ensure_current().is_ok() {
            self.traversal.insert(id, attempt);
        } else {
            attempt.cancel();
        }
        match result {
            Err(
                Error::Authentication
                | Error::Authority
                | Error::Capacity
                | Error::Unavailable
                | Error::Underlay,
            ) => Ok(()),
            result => result,
        }
    }

    fn traversal_message(
        &mut self,
        attempt: &mut Attempt,
        remote: SocketAddr,
        kind: u8,
        body: &[u8],
    ) -> Result<(), Error> {
        let Some(active) = attempt.active.clone() else {
            return Ok(());
        };
        if !attempt.completed
            && super::super::traversal::matching_endpoint(&self.shared, &attempt.offer)
                != Some(self.id)
        {
            attempt.cancel();
            return Ok(());
        }
        if active.ensure_current().is_err() || attempt.offer.ensure_current().is_err() {
            return Ok(());
        }
        let plan = self.shared.plan.clone();
        let cfg = &*plan;
        let peer = attempt.offer.peer().candidate.socket.node_id;
        let carrier = attempt.offer.offer().carrier.clone();
        let dialer = carrier.dialer == cfg.node;
        if kind == 0 {
            adjacency::incoming_bound(cfg, peer, &carrier, attempt.offer.offer().digest, body)?;
            return Ok(());
        }
        if attempt.completed {
            return Ok(());
        }
        if kind == 1 && !dialer {
            let initiation =
                adjacency::incoming_bound(cfg, peer, &carrier, attempt.offer.offer().digest, body)?;
            if let Some(previous) = &attempt.transcript {
                if previous.client != initiation.client {
                    return Err(Error::Authentication);
                }
                return Ok(());
            }
            let transcript = initiation.respond()?;
            attempt.output(
                2,
                transcript.message(&self.shared.identity, Kind::Challenge)?,
            );
            attempt.transcript = Some(transcript);
            return Ok(());
        }
        if kind == 2 && dialer {
            let transcript = attempt
                .initiation
                .as_ref()
                .ok_or(Error::Authentication)?
                .challenge(cfg, peer, body)?;
            if let Some(previous) = &attempt.transcript {
                if previous.id() != transcript.id() {
                    return Err(Error::Authentication);
                }
                return Ok(());
            }
            attempt.output(3, transcript.message(&self.shared.identity, Kind::Ack)?);
            attempt.transcript = Some(transcript);
            return Ok(());
        }
        if (kind == 3 && !dialer) || (kind == 4 && dialer) {
            let transcript = attempt.transcript.as_ref().ok_or(Error::Authentication)?;
            transcript.verify(
                cfg,
                peer,
                if dialer { Kind::Ready } else { Kind::Ack },
                body,
            )?;
            let ready = if dialer {
                None
            } else {
                Some(transcript.message(&self.shared.identity, Kind::Ready)?)
            };
            active.ensure_current().map_err(|_| Error::Authority)?;
            attempt
                .offer
                .ensure_current()
                .map_err(|_| Error::Authority)?;
            // The authenticated offer's remote participant is the authority for
            // provenance. Never relabel a channel using current State candidates
            // or a coincidentally equal published address.
            let participant = attempt.offer.peer();
            if meshway_control::connectivity::udp_recipient(&participant.address)
                .map_err(|_| Error::Authentication)?
                != remote
            {
                return Err(Error::Authentication);
            }
            let acquisition = LinkAcquisition::CoordinatedUdp {
                attempt_id: attempt.offer.offer().attempt_id,
                endpoint_source: participant.source,
            };
            let transcript = attempt.transcript.take().ok_or(Error::Authentication)?;
            let capacity = attempt.capacity.take().ok_or(Error::Capacity)?;
            if self.promote_scoped(
                peer,
                carrier,
                remote,
                transcript,
                capacity,
                attempt.offer.expires_at().into(),
                attempt.replacement,
                acquisition,
                Some(active),
            )? {
                attempt.completed = true;
                if let Some(ready) = ready {
                    attempt.output(4, ready);
                } else {
                    attempt.output.clear();
                }
            } else {
                attempt.cancel();
            }
        }
        Ok(())
    }
}
