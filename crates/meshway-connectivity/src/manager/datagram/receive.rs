//! Receive demultiplexing: control before business frames, and bounded
//! dispatch to the event queues.
use super::*;

impl Owner {
    async fn receive(
        &mut self,
        frame: Frame,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("manager.udp.dispatch.poll");
        PROFILE.poll(self.receive_inner(frame, received_at)).await
    }

    pub(super) async fn receive_batch(
        &mut self,
        frames: Vec<Frame>,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        // Demultiplex before the business event queue. Its saturation must not
        // delay a health/handshake/STUN response already received by this socket.
        // Signature work retains the independent per-peer/global control budget.
        let (control, data): (Vec<_>, Vec<_>) = frames.into_iter().partition(|frame| {
            frame.payload.starts_with(adjacency::MAGIC)
                || frame.payload.starts_with(traversal::MAGIC)
                || meshway_stun::classify(&frame.payload).is_some()
        });
        for frame in control {
            self.receive(frame, received_at).await?;
        }
        // Under backlog, give the notified consumer a turn before another
        // burst, rather than waiting until all 1,024 slots are occupied. Small
        // ready prefixes are published immediately; never await capacity.
        if self.shared.events.consumer_turn(data.len()) {
            meshway_logging::profile::event("manager.udp.dispatch", "consumer_turn");
            tokio::task::yield_now().await;
        }
        let mut started = std::time::Instant::now();
        let mut checked = BTreeMap::new();
        let mut events = Vec::new();
        for frame in data {
            if started.elapsed() >= Duration::from_micros(200) {
                // Descheduling is not packet congestion. Retain the bounded
                // already-read suffix across a fair turn, then recheck its
                // original channel when dispatch resumes.
                meshway_logging::profile::event("manager.udp.dispatch", "quantum_yield");
                self.publish(std::mem::take(&mut events));
                tokio::task::yield_now().await;
                started = std::time::Instant::now();
                checked.clear();
            }
            let Some(remote) = frame.peer else {
                continue;
            };
            let Some(channel) = self.channels.get(&remote) else {
                continue;
            };
            if *checked
                .entry(remote)
                .or_insert_with(|| self.selected(channel))
            {
                let event = Event::Frame {
                    link: Box::new(channel.fact.clone()),
                    frame,
                    received_at,
                };
                if Shared::business_event(&event) {
                    meshway_logging::profile::event("manager.udp", "mesh_frame");
                    events.push(event);
                } else {
                    self.dispatch_event(event);
                }
            }
        }
        self.publish(events);
        // One fair scheduling turn per bounded batch, rather than one per
        // rejected business packet while control waits in the socket backlog.
        tokio::task::yield_now().await;
        Ok(())
    }

    async fn receive_inner(
        &mut self,
        frame: Frame,
        received_at: std::time::Instant,
    ) -> Result<(), Error> {
        let Some(remote) = frame.peer else {
            return Ok(());
        };
        if meshway_stun::classify(&frame.payload).is_some() {
            self.stun_receive(remote, &frame.payload)?;
            return Ok(());
        }
        if frame.payload.starts_with(traversal::MAGIC) {
            self.traversal_receive(remote, &frame.payload).await?;
            return Ok(());
        }
        if frame.payload.starts_with(adjacency::MAGIC) {
            // A single bounded rate budget covers all listeners/candidates and
            // is checked before signature work or attacker-selected allocation.
            let admitted = match self.channels.get_mut(&remote) {
                Some(channel) => channel.budget.take(),
                None => self
                    .shared
                    .control_budget
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .take(),
            };
            if !admitted {
                return Ok(());
            }
            let Ok((kind, body)) = adjacency::decode(&frame.payload) else {
                return Ok(());
            };
            match kind {
                Kind::Init => self.init(remote, body).await?,
                Kind::Challenge | Kind::Ready => self.client_message(remote, kind, body).await?,
                Kind::Ack => self.ack(remote, body).await?,
                Kind::Ping | Kind::Pong => self.health(remote, kind, body).await?,
                Kind::Probe => {
                    self.answer_size_probe(remote, frame.payload.len(), body)
                        .await?
                }
            }
        } else if let Some(channel) = self.channels.get(&remote)
            && self.selected(channel)
        {
            // Source is a demultiplex key only after signed challenge completion.
            // SecureNode remains the sole packet decoder and proof verifier.
            self.dispatch(channel, frame, received_at);
        }
        Ok(())
    }

    fn dispatch(&self, channel: &Channel, frame: Frame, received_at: std::time::Instant) {
        let event = Event::Frame {
            link: Box::new(channel.fact.clone()),
            frame,
            received_at,
        };
        self.dispatch_event(event);
    }

    fn dispatch_event(&self, event: Event) {
        meshway_logging::profile::event("manager.udp", "mesh_frame");
        match self.shared.event_sender(&event).try_send(event) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                meshway_logging::profile::event("manager.udp.drop", "event_full");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                meshway_logging::profile::event("manager.udp.drop", "event_closed");
            }
        }
    }

    /// Queue this read batch's business events behind any still waiting and
    /// publish what the event queue has room for.
    fn publish(&mut self, events: Vec<Event>) {
        self.backlog.events.extend(events);
        self.flush();
    }

    /// Publish the backlog in groups of at most 32. A group the queue has no
    /// room for waits, with the rest behind it, and [`Backlog::blocked`]
    /// stops reading the socket until [`Owner::make_room`] or the stall bound.
    /// While shedding, as before this bound, the part without room is dropped.
    pub(super) fn flush(&mut self) {
        while !self.backlog.events.is_empty() {
            let mut count = self.backlog.events.len().min(32);
            if self.backlog.shedding {
                count = count.min(self.shared.events.capacity());
                if count == 0 {
                    let rest = std::mem::take(&mut self.backlog.events);
                    self.shed(rest.len(), "event_full");
                    break;
                }
            }
            let group: Vec<_> = self.backlog.events.drain(..count).collect();
            match self.shared.events.try_send_batch(group) {
                Ok(()) => {
                    meshway_logging::profile::event("manager.event_queue", "publish_batch");
                    self.backlog.accepted();
                    self.backlog.since = None;
                }
                Err(mpsc::error::TrySendError::Full(group)) if !self.backlog.shedding => {
                    for event in group.into_iter().rev() {
                        self.backlog.events.push_front(event);
                    }
                    meshway_logging::profile::event("manager.udp", "backpressure");
                    self.backlog.since.get_or_insert_with(Instant::now);
                    return;
                }
                Err(mpsc::error::TrySendError::Full(group)) => self.shed(group.len(), "event_full"),
                Err(mpsc::error::TrySendError::Closed(group)) => {
                    let rest = std::mem::take(&mut self.backlog.events);
                    self.shed(group.len() + rest.len(), "event_closed");
                }
            }
        }
        self.backlog.since = None;
    }

    /// The room awaited for the backlog's first group is now held.
    pub(super) fn make_room(&mut self, reservation: Option<super::super::events::Reservation>) {
        let Some(reservation) = reservation else {
            // Closed: the Manager is stopping.
            let rest = std::mem::take(&mut self.backlog.events);
            self.shed(rest.len(), "event_closed");
            self.backlog.since = None;
            return;
        };
        let count = self.backlog.events.len().min(32);
        let group: Vec<_> = self.backlog.events.drain(..count).collect();
        match self.shared.events.send_reserved(reservation, group) {
            Ok(()) => {
                meshway_logging::profile::event("manager.event_queue", "publish_batch");
                self.backlog.accepted();
                self.backlog.since = None;
                self.flush();
            }
            Err(mpsc::error::TrySendError::Full(group)) => {
                // Not this group's reservation; wait again for the same group.
                for event in group.into_iter().rev() {
                    self.backlog.events.push_front(event);
                }
            }
            Err(mpsc::error::TrySendError::Closed(group)) => {
                let rest = std::mem::take(&mut self.backlog.events);
                self.shed(group.len() + rest.len(), "event_closed");
                self.backlog.since = None;
            }
        }
    }

    /// The consumer made no room within [`RECEIVE_STALL`]: control frames
    /// behind this backlog in the socket must not wait for it any longer.
    pub(super) fn stalled(&mut self) {
        meshway_logging::profile::event("manager.udp", "receive_stall");
        self.backlog.shedding = true;
        self.backlog.shed_since = Some(Instant::now());
        self.flush();
    }

    /// Count `count` business events already taken from the backlog as dropped.
    fn shed(&mut self, count: usize, reason: &'static str) {
        if self.backlog.shedding {
            self.backlog.dropped = self.backlog.dropped.saturating_add(count as u64);
        }
        for _ in 0..count {
            meshway_logging::profile::event("manager.udp.drop", reason);
        }
    }
}

/// Business events of the last read batch that the Manager event queue had no
/// room for. While any wait, this owner reads nothing more from its socket:
/// the backlog stays within one receive batch and further arrivals wait in
/// the kernel receive buffer, whose overflow the kernel counts.
#[derive(Default)]
pub(super) struct Backlog {
    events: VecDeque<Event>,
    /// When the first waiting group found the queue full.
    since: Option<Instant>,
    /// The consumer made no room for [`RECEIVE_STALL`]: read again and drop
    /// what has no room until the queue accepts a group again.
    shedding: bool,
    shed_since: Option<Instant>,
    dropped: u64,
}

impl Backlog {
    /// Reading stops while a group waits for room, unless shedding.
    pub(super) fn blocked(&self) -> bool {
        !self.events.is_empty() && !self.shedding
    }

    /// The first group still waiting for room, at most 32 events.
    pub(super) fn waiting(&self) -> impl Iterator<Item = &Event> {
        self.events.iter().take(32)
    }

    /// When waiting for room ends in shedding.
    pub(super) fn stall_at(&self) -> Option<Instant> {
        self.since.map(|since| since + RECEIVE_STALL)
    }

    fn accepted(&mut self) {
        if !self.shedding {
            return;
        }
        self.shedding = false;
        let since = self.shed_since.take();
        let dropped = std::mem::take(&mut self.dropped);
        if dropped != 0 && diagnostic_event() {
            tracing::warn!(
                dropped,
                stalled_ms = since.map_or(0, |since| since.elapsed().as_millis() as u64),
                "Connectivity event queue made no room; business frames dropped while it was stalled"
            );
        }
    }
}
