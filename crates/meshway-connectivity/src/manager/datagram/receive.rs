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
                self.dispatch_batch(std::mem::take(&mut events));
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
        self.dispatch_batch(events);
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

    fn dispatch_batch(&self, mut events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        let count = events.len();
        let accepted = count.min(self.shared.events.capacity());
        events.truncate(accepted);
        let mut dropped = count - accepted;
        let mut reason = "event_full";
        if accepted != 0 {
            match self.shared.events.try_send_batch(events) {
                Ok(()) => meshway_logging::profile::event("manager.event_queue", "publish_batch"),
                Err(error) => {
                    if matches!(error, mpsc::error::TrySendError::Closed(_)) {
                        reason = "event_closed";
                    }
                    dropped += accepted;
                }
            }
        }
        for _ in 0..dropped {
            meshway_logging::profile::event("manager.udp.drop", reason);
        }
    }
}
