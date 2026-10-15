//! Bounded glue between the unique SecureNode flow owner and Host kernel owner.
//! The resource registry records only projection ACK/CT confirmation, never
//! packet tuples, business admission or reverse-path routing.
mod driver;
mod scope;

use meshway_dataplane::{PendingAction, exit::ExitProjection, secure::SecureNode};
use meshway_protocol::ipc::Problem;
use meshway_underlay::{ExitReplyReader, ExitRole};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

/// Work items (projections, packets, barriers) the driver FIFO holds.
const CAPACITY: usize = 128;
/// Slots packets of installed flows leave free: projections and the first
/// packets of new flows still find room beside a saturating flow.
const RESERVED: usize = CAPACITY / 4;

enum Work {
    Projection(Box<ExitProjection>),
    Packet(Box<PendingAction>),
    Barrier(tokio::sync::oneshot::Sender<()>),
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Health {
    pub flows: u32,
    pub failed: bool,
    /// The role's Host owner stopped serving by itself (its own kernel
    /// failure or an exit-owned object removed outside it), not on this
    /// runtime's request.
    pub ended: bool,
}

pub(super) struct Runtime {
    pub plan: meshway_configuration::netmap::ExitRole,
    pub reader: ExitReplyReader,
    role: ExitRole,
    sender: mpsc::Sender<Work>,
    failures: mpsc::Receiver<meshway_dataplane::exit::FlowIncarnation>,
    pub health: watch::Receiver<Health>,
    stop: CancellationToken,
    /// Packets dropped for want of queue room since the role started.
    dropped: u64,
    _on_drop: tokio_util::sync::DropGuard,
    task: tokio::task::JoinHandle<Result<(), Problem>>,
}

impl Runtime {
    pub fn start(
        role: ExitRole,
        plan: meshway_configuration::netmap::ExitRole,
    ) -> Result<Self, Problem> {
        let reader = role.reply_reader().map_err(|_| Problem::Underlay)?;
        let (sender, received) = mpsc::channel(CAPACITY);
        let (failed, failures) = mpsc::channel(CAPACITY);
        let (status, health) = watch::channel(Health::default());
        let stop = CancellationToken::new();
        let task = tokio::spawn(driver::run(
            role.clone(),
            received,
            failed,
            status,
            stop.clone(),
        ));
        Ok(Self {
            plan,
            reader,
            role,
            sender,
            failures,
            health,
            _on_drop: stop.clone().drop_guard(),
            stop,
            dropped: 0,
            task,
        })
    }

    /// Called before enqueueing packets and after admission/reconcile/maintenance.
    /// Reserve queue capacity before claiming X2's move-only completion callback.
    pub fn drive(&mut self, secure: &mut SecureNode) -> bool {
        while let Ok(flow) = self.failures.try_recv() {
            secure.fail_exit_projection(&flow);
        }
        for _ in 0..CAPACITY {
            let Ok(slot) = self.sender.try_reserve() else {
                return false;
            };
            let Some(event) = secure.next_exit_projection() else {
                return true;
            };
            slot.send(Work::Projection(Box::new(event)));
        }
        false
    }

    /// All operations created by this admission precede its packet in this
    /// single FIFO. Without room the packet alone is dropped, as a full
    /// interface queue would: its flow, projection and tuple stay live (no
    /// Retire, no tombstone) and the next packet is admitted as usual. The
    /// owner never waits here while holding SecureNode.
    pub fn packet(&mut self, secure: &mut SecureNode, packet: PendingAction) {
        let drained = self.drive(secure);
        // A packet whose projection is already acknowledged belongs to an
        // installed flow; one still waiting for its ACK starts a new flow.
        let installed = packet.ensure_current().is_ok();
        if !admit(
            &self.sender,
            drained,
            installed,
            Work::Packet(Box::new(packet)),
        ) {
            self.dropped = self.dropped.saturating_add(1);
            meshway_logging::profile::event("exit.drop", "queue_capacity");
            if self.dropped.is_power_of_two() {
                tracing::warn!(
                    dropped = self.dropped,
                    "Exit queue full; packet dropped, its flow kept"
                );
            }
        }
    }

    pub async fn stop(mut self, secure: &mut SecureNode) -> Result<(), Problem> {
        // Cancel every original queued write first; cleanup itself does not
        // require live authority. Kernel operations already admitted are joined.
        self.stop.cancel();
        secure.retire_exits();
        loop {
            while let Some(event) = secure.next_exit_projection() {
                if self
                    .sender
                    .send(Work::Projection(Box::new(event)))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            // In-flight callbacks reserve X2 command slots. Wait for the FIFO
            // to settle, then materialize any retirements previously budgeted
            // behind them. A single next()==None is not a complete drain.
            if self.barrier().await.is_err() {
                break;
            }
            let Some(event) = secure.next_exit_projection() else {
                break;
            };
            if self
                .sender
                .send(Work::Projection(Box::new(event)))
                .await
                .is_err()
            {
                break;
            }
        }
        drop(self.sender);
        while self.failures.try_recv().is_ok() {}
        self.task.await.map_err(|_| Problem::Worker)?
    }

    async fn barrier(&self) -> Result<(), Problem> {
        let (reply, done) = tokio::sync::oneshot::channel();
        self.sender
            .send(Work::Barrier(reply))
            .await
            .map_err(|_| Problem::Worker)?;
        done.await.map_err(|_| Problem::Worker)
    }

    pub async fn synchronize(&mut self, secure: &mut SecureNode) -> Result<(), Problem> {
        loop {
            while let Ok(flow) = self.failures.try_recv() {
                secure.fail_exit_projection(&flow);
            }
            while let Some(event) = secure.next_exit_projection() {
                self.sender
                    .send(Work::Projection(Box::new(event)))
                    .await
                    .map_err(|_| Problem::Worker)?;
            }
            self.barrier().await?;
            while let Ok(flow) = self.failures.try_recv() {
                secure.fail_exit_projection(&flow);
            }
            let Some(event) = secure.next_exit_projection() else {
                break;
            };
            self.sender
                .send(Work::Projection(Box::new(event)))
                .await
                .map_err(|_| Problem::Worker)?;
        }
        if !self.role.is_active() || self.health.borrow().failed {
            return Err(Problem::Underlay);
        }
        Ok(())
    }
}

/// Queue one packet without waiting. Packets of installed flows leave
/// `RESERVED` slots to projections and new flows' first packets, so neither
/// a saturating flow nor a burst of new flows starves the other.
fn admit<T>(sender: &mpsc::Sender<T>, drained: bool, installed: bool, item: T) -> bool {
    let room = sender.capacity() > if installed { RESERVED } else { 0 };
    drained && room && sender.try_send(item).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_drops_the_packet_and_accepts_the_next_once_drained() {
        let (sender, mut received) = mpsc::channel(CAPACITY);
        for item in 0..CAPACITY - RESERVED {
            assert!(admit(&sender, true, true, item));
        }
        // An installed flow's packet finds the reserve closed to it ...
        assert!(!admit(&sender, true, true, usize::MAX));
        // ... while new flows' first packets still enter.
        for item in 0..RESERVED {
            assert!(admit(&sender, true, false, item));
        }
        assert!(!admit(&sender, true, false, usize::MAX));
        // Projections not yet queued come first: the packet waits for none.
        assert!(received.try_recv().is_ok());
        assert!(!admit(&sender, false, false, usize::MAX));
        while received.try_recv().is_ok() {}
        // The next packet of the same flow is queued as usual.
        assert!(admit(&sender, true, true, 7));
        assert_eq!(received.try_recv().ok(), Some(7));
    }
}
