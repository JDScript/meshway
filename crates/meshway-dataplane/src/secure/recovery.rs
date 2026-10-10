//! Exact, scope-frozen handshake retransmission; never another Noise owner.
use super::{Error, Guard, PendingFrame};
use std::time::{Duration, Instant};

const INTERVAL: Duration = Duration::from_millis(250);
// Covers the largest supported 60s handshake deadline without an unbounded
// duplicate-triggered response stream. Initial output consumes one slot too.
const MAX_OUTPUTS: u16 = 256;
// Candidate-v4's binary IDs, eight contributions and seven maximum-size
// handshake proofs fit in 4KiB (codec boundary regression).
// At most two stages per circuit retain an output; circuit capacity bounds the
// total cache, and maintain releases bytes at the original handshake deadline.
const MAX_BYTES: usize = 4096;

struct Budget {
    next: Instant,
    outputs: u16,
}
impl Budget {
    fn new(now: Instant) -> Self {
        Self {
            next: now + INTERVAL,
            outputs: 1,
        }
    }

    fn take(&mut self, now: Instant) -> bool {
        if now < self.next || self.outputs >= MAX_OUTPUTS {
            return false;
        }
        self.outputs += 1;
        self.next = now + INTERVAL;
        true
    }
}

pub(super) struct Stage {
    pub fingerprint: [u8; 32],
    guard: Guard,
    output: Option<PendingFrame>,
    budget: Budget,
}

impl Stage {
    pub fn new(
        fingerprint: [u8; 32],
        guard: Guard,
        output: Option<&PendingFrame>,
    ) -> Result<Self, Error> {
        if output.is_some_and(|frame| frame.bytes.len() > MAX_BYTES) {
            return Err(Error::Capacity);
        }
        guard.finish()?;
        if let Some(frame) = output {
            frame.guard.finish()?;
        }
        Ok(Self {
            fingerprint,
            guard,
            output: output.map(copy),
            // Pacing begins when the original output is retained, not at an
            // earlier operation entry (a slow handshake cannot mint a retry).
            budget: Budget::new(Instant::now()),
        })
    }

    pub fn ensure_current(&self) -> Result<(), Error> {
        self.guard.ensure_current()?;
        if let Some(frame) = &self.output {
            frame.ensure_current()?;
        }
        Ok(())
    }

    pub fn output(&self) -> Option<&PendingFrame> {
        self.output.as_ref()
    }

    pub fn retry_at(&self) -> Result<Option<Instant>, Error> {
        self.ensure_current()?;
        Ok(
            (self.output.is_some() && self.budget.outputs < MAX_OUTPUTS).then_some(
                self.guard
                    .expires_at()
                    .map_or(self.budget.next, |at| self.budget.next.min(at)),
            ),
        )
    }

    pub fn retry(&mut self) -> Result<Option<PendingFrame>, Error> {
        self.ensure_current()?;
        let Some(output) = &self.output else {
            return Ok(None);
        };
        if !self.budget.take(Instant::now()) {
            return Ok(None);
        }
        // Copy the entire original signed frame and guard. Neither the caller
        // nor a later publication can extend this operation's authority.
        let pending = copy(output);
        pending.ensure_current()?;
        Ok(Some(pending))
    }
}

fn copy(frame: &PendingFrame) -> PendingFrame {
    PendingFrame {
        bytes: frame.bytes.payload().into(),
        next: frame.next,
        carrier: frame.carrier.clone(),
        circuit: frame.circuit,
        guard: frame.guard.clone(),
    }
}

#[derive(Default)]
pub(super) struct Recovery {
    pub init: Option<Stage>,
    pub response: Option<Stage>,
}

impl Recovery {
    pub fn maintain(&mut self) {
        // No cached operation crosses a scope replacement, even if its circuit
        // is legally retained. Reconcile never rebinds these original guards.
        for stage in [&mut self.init, &mut self.response] {
            if stage
                .as_ref()
                .is_some_and(|stage| stage.ensure_current().is_err())
            {
                *stage = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_budget_is_paced_bounded_and_does_not_catch_up_after_a_late_poll() {
        let start = Instant::now();
        let mut budget = Budget::new(start);
        assert!(!budget.take(start));
        assert!(!budget.take(start + INTERVAL / 2));
        let late = start + INTERVAL * 4;
        assert!(budget.take(late));
        assert!(!budget.take(late));
        assert_eq!(budget.next, late + INTERVAL);
        for _ in 2..MAX_OUTPUTS {
            assert!(budget.take(budget.next));
        }
        assert!(!budget.take(budget.next + Duration::from_secs(60)));
        assert_eq!(budget.outputs, MAX_OUTPUTS);
    }
}
