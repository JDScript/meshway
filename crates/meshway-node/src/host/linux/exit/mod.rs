//! Bounded glue between the unique SecureNode flow owner and Host kernel owner.
//! The resource registry records only projection ACK/CT confirmation, never
//! packet tuples, business admission or reverse-path routing.
mod driver;
mod lanes;
mod scope;

use meshway_dataplane::{PendingAction, exit::ExitProjection, secure::SecureNode};
use meshway_protocol::ipc::Problem;
use meshway_underlay::{ExitReplyReader, ExitRole};
use scope::PacketGuard;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

impl crate::host::ExitDriver for Runtime {
    fn drive(&mut self, secure: &mut SecureNode) -> bool {
        Runtime::drive(self, secure)
    }
    fn packet(&mut self, secure: &mut SecureNode, packet: PendingAction) {
        Runtime::packet(self, secure, packet);
    }
    fn synchronize<'a>(
        &'a mut self,
        secure: &'a mut SecureNode,
    ) -> crate::host::Local<'a, Result<(), Problem>> {
        Box::pin(Runtime::synchronize(self, secure))
    }
    fn stop<'a>(
        self: Box<Self>,
        secure: &'a mut SecureNode,
    ) -> crate::host::Local<'a, Result<(), Problem>> {
        Box::pin(Runtime::stop(*self, secure))
    }
    fn recv_batch(
        &mut self,
        max: usize,
    ) -> crate::host::Local<'_, crate::host::Result<Vec<Vec<u8>>>> {
        Box::pin(async move {
            self.reader
                .recv_batch(max)
                .await
                .map(|replies| replies.into_iter().map(|reply| reply.packet).collect())
                .map_err(Into::into)
        })
    }
}

/// Work items (projections, packets, barriers) the driver FIFO holds.
const CAPACITY: usize = 128;
/// Slots packets of installed flows leave free: projections and the first
/// packets of new flows still find room beside a saturating flow.
const RESERVED: usize = CAPACITY / 4;
/// Direct writes one batch to the exit TUN's owner carries at most (the
/// packet owner's turn).
const OUTBOX: usize = 32;

type Lanes = lanes::Lanes<meshway_dataplane::exit::FlowIncarnation>;

enum Work {
    Projection(Box<ExitProjection>),
    /// A packet the driver releases, with its place in the flow's lane.
    Packet(
        Box<PendingAction>,
        Option<lanes::Queued<meshway_dataplane::exit::FlowIncarnation>>,
    ),
    Barrier(tokio::sync::oneshot::Sender<()>),
}

use crate::host::ExitHealth as Health;

pub(super) struct Runtime {
    pub reader: ExitReplyReader,
    role: ExitRole,
    sender: mpsc::Sender<Work>,
    failures: mpsc::Receiver<meshway_dataplane::exit::FlowIncarnation>,
    pub health: watch::Receiver<Health>,
    stop: CancellationToken,
    lanes: Lanes,
    /// Writes of installed flows taken in this turn, submitted as one batch.
    outbox: Vec<meshway_underlay::ExitWrite>,
    /// Packets dropped for want of queue room since the role started.
    dropped: u64,
    _on_drop: tokio_util::sync::DropGuard,
    task: tokio::task::JoinHandle<Result<(), Problem>>,
}

impl Runtime {
    pub fn start(role: ExitRole) -> Result<Self, Problem> {
        let reader = role.reply_reader().map_err(|_| Problem::Underlay)?;
        let (sender, received) = mpsc::channel(CAPACITY);
        let (failed, failures) = mpsc::channel(CAPACITY);
        let (status, health) = watch::channel(Health::default());
        let stop = CancellationToken::new();
        let lanes = Lanes::default();
        let task = tokio::spawn(driver::run(
            role.clone(),
            received,
            failed,
            status,
            stop.clone(),
            lanes.clone(),
        ));
        Ok(Self {
            reader,
            role,
            sender,
            failures,
            health,
            _on_drop: stop.clone().drop_guard(),
            stop,
            lanes,
            outbox: Vec::with_capacity(OUTBOX),
            dropped: 0,
            task,
        })
    }

    /// Called after a turn of packets and after admission/reconcile/maintenance:
    /// submits the turn's direct writes, then drives projections.
    pub fn drive(&mut self, secure: &mut SecureNode) -> bool {
        self.flush();
        self.projections(secure)
    }

    /// Reserve queue capacity before claiming X2's move-only completion callback.
    fn projections(&mut self, secure: &mut SecureNode) -> bool {
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
    ///
    /// A packet of a flow the driver already released one packet of, whose
    /// own projection is acknowledged and still live at the exit TUN's
    /// owner, and with no packet of its flow in the FIFO, is queued for that
    /// owner's next batch instead: no driver turn, no awaited write.
    pub fn packet(&mut self, secure: &mut SecureNode, packet: PendingAction) {
        // A direct write creates and needs no projection operation; the
        // turn's operations are driven at its end (`drive`).
        if let Some(flow) = packet.exit_flow()
            && self.lanes.direct(&flow)
            && let Some(writer) = self
                .role
                .writer(meshway_underlay::ExitIncarnation::new(flow.bytes()))
            && let Ok(scope) = packet.exit_scope()
        {
            let released = packet.release(|action| match action {
                meshway_dataplane::Action::Exit { packet, .. } => Some(packet.into_bytes()),
                _ => None,
            });
            if let Ok(Some(bytes)) = released {
                let guard = PacketGuard {
                    scope,
                    stop: self.stop.clone(),
                };
                if let Ok(write) = writer.write(bytes, guard) {
                    self.outbox.push(write);
                }
                if self.outbox.len() >= OUTBOX {
                    self.flush();
                }
            }
            return;
        }
        let drained = self.projections(secure);
        // A packet whose projection is already acknowledged belongs to an
        // installed flow; one still waiting for its ACK starts a new flow.
        let installed = packet.ensure_current().is_ok();
        let flow = packet.exit_flow();
        // Writes taken before this packet go first.
        self.flush();
        let queued = flow.and_then(|flow| self.lanes.enqueue(flow));
        if !admit(
            &self.sender,
            drained,
            installed,
            Work::Packet(Box::new(packet), queued),
        ) {
            self.full(1);
        }
    }

    /// Submit the turn's direct writes as one batch; without room they are
    /// dropped as a full FIFO drops a packet.
    fn flush(&mut self) {
        if self.outbox.is_empty() {
            return;
        }
        let writes = std::mem::replace(&mut self.outbox, Vec::with_capacity(OUTBOX));
        let count = writes.len() as u64;
        if let Err(meshway_underlay::Error::Capacity) = self.role.submit_writes(writes) {
            self.full(count);
        }
    }

    fn full(&mut self, count: u64) {
        let before = self.dropped;
        self.dropped = self.dropped.saturating_add(count);
        meshway_logging::profile::event("exit.drop", "queue_capacity");
        // One warn per power of two crossed (bounded).
        if before.checked_ilog2() != self.dropped.checked_ilog2() {
            tracing::warn!(
                dropped = self.dropped,
                "Exit queue full; packet dropped, its flow kept"
            );
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
