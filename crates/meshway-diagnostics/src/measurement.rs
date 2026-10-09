use super::*;
use meshway_model::{
    Version,
    observations::{ObservationSample, ProbeChannels, ProbeWindow},
    state::{ProbeKind, ProbeObservation, ProbeOutcome},
};

pub(super) struct Measurement {
    pub id: EventId,
    pub challenge: Digest,
    pub deadline: Instant,
    started: Instant,
    started_at: DateTime<Utc>,
    pub send_completed: bool,
    pub receipt: Option<(ProbeChannel, Instant)>,
    pub reply: Option<(ProbeChannel, Instant)>,
    pub local_failure: bool,
}

impl Measurement {
    pub fn new(task: &Task, now: Instant) -> Result<Self, Error> {
        let mut random = [0; 48];
        getrandom::fill(&mut random).map_err(|_| Error::Random)?;
        let id = EventId::new(
            uuid::Builder::from_random_bytes(random[..16].try_into().map_err(|_| Error::Random)?)
                .into_uuid(),
        )
        .map_err(|_| Error::Random)?;
        let challenge: [u8; 32] = random[16..].try_into().map_err(|_| Error::Random)?;
        let ms = i64::try_from(task.snapshot.now_ms().map_err(|_| Error::Binding)?)
            .map_err(|_| Error::Binding)?;
        let started_at = DateTime::from_timestamp_millis(ms)
            .ok_or(Error::Binding)?
            .max(task.last_end.unwrap_or(DateTime::<Utc>::MIN_UTC));
        // A netmap task is bounded by its own deadline and the lease alone.
        let permit_expiry = if task.netmap {
            u64::MAX
        } else {
            task.snapshot
                .configuration()
                .map_err(|_| Error::Binding)?
                .diagnostics[&task.task.id]
                .expires_at_ms
        };
        let expiry = u64::try_from(task.task.expires_at.timestamp_millis())
            .map_err(|_| Error::Binding)?
            .min(permit_expiry);
        let deadline = (now + DEADLINE)
            .min(task.snapshot.expires_at())
            .min(now + Duration::from_millis(expiry.saturating_sub(ms as u64)));
        Ok(Self {
            id,
            challenge: challenge.into(),
            deadline,
            started: now,
            started_at,
            send_completed: false,
            receipt: None,
            reply: None,
            local_failure: false,
        })
    }

    pub fn finish(self, task: &Task, now: Instant) -> ObservationSample {
        let timing = self.receipt.as_ref().zip(self.reply.as_ref()).and_then(
            |((_, sent), (_, received))| {
                (*received <= self.deadline && *sent <= self.deadline)
                    .then(|| received.checked_duration_since(*sent))
                    .flatten()
            },
        );
        let local_failure = self.local_failure || (self.reply.is_some() && timing.is_none());
        let sent = self
            .receipt
            .as_ref()
            .is_some_and(|(_, at)| *at >= self.started && *at <= self.deadline)
            && !local_failure;
        let outcome = if local_failure || !sent {
            ProbeOutcome::Failed
        } else if timing.is_some() {
            ProbeOutcome::Success
        } else {
            ProbeOutcome::Timeout
        };
        let completed = match outcome {
            ProbeOutcome::Success => self.reply.as_ref().map(|(_, at)| *at).unwrap_or(now),
            ProbeOutcome::Timeout => self.deadline,
            _ => now.min(self.deadline),
        };
        let elapsed = completed
            .saturating_duration_since(self.started)
            .max(Duration::from_micros(1));
        let end = self.started_at
            + chrono::Duration::from_std(elapsed).unwrap_or(chrono::Duration::seconds(5));
        let probe_started_at = self
            .receipt
            .as_ref()
            .filter(|_| sent)
            .and_then(|(_, at)| {
                chrono::Duration::from_std(at.saturating_duration_since(self.started)).ok()
            })
            .map_or(self.started_at, |elapsed| self.started_at + elapsed);
        let counts = task
            .task
            .metrics
            .contains(&MetricKind::ProbeLoss)
            .then_some(ProbeWindow {
                sent: u32::from(sent),
                replied_within_deadline: u32::from(outcome == ProbeOutcome::Success),
                timeout: u32::from(outcome == ProbeOutcome::Timeout),
                pending: 0,
            });
        let channels = self.receipt.map(|(request, _)| ProbeChannels {
            request,
            reply: self.reply.map(|(channel, _)| channel),
        });
        ObservationSample {
            measurement_id: self.id,
            version: Version::MIN,
            window_start: self.started_at,
            window_end: end,
            partial: false,
            traffic: None,
            probe_counts: counts,
            probe: Some(ProbeObservation {
                measurement_id: self.id,
                kind: ProbeKind::Path,
                link_id: None,
                transport: None,
                channels,
                path: task.path.clone(),
                publication_id: task.task.configuration.execution_publication,
                started_at: probe_started_at,
                completed_at: Some(end),
                outcome,
                duration_us: timing
                    .filter(|_| outcome == ProbeOutcome::Success)
                    .and_then(|time| u64::try_from(time.as_micros()).ok())
                    .and_then(|us| Sequence::new(us).ok()),
            }),
        }
    }
}
