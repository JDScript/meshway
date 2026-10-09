//! Bounded glue between the unique SecureNode flow owner and Host kernel owner.
//! The resource registry records only projection ACK/CT confirmation, never
//! packet tuples, business admission or reverse-path routing.
mod driver;
mod scope;
#[cfg(test)]
pub(super) mod testing;
#[cfg(test)]
mod tests;

use meshway_dataplane::{PendingAction, exit::ExitProjection, secure::SecureNode};
use meshway_ipc::Problem;
use meshway_underlay::{ExitReplyReader, ExitRole};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

const CAPACITY: usize = 128;

enum Work {
    Projection(Box<ExitProjection>),
    Packet(Box<PendingAction>),
    Barrier(tokio::sync::oneshot::Sender<()>),
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Health {
    pub flows: u32,
    pub failed: bool,
}

pub(super) struct Runtime {
    pub plan: meshway_configuration::execution::ExitRolePlan,
    pub reader: ExitReplyReader,
    role: ExitRole,
    sender: mpsc::Sender<Work>,
    failures: mpsc::Receiver<meshway_dataplane::exit::FlowIncarnation>,
    pub health: watch::Receiver<Health>,
    stop: CancellationToken,
    _on_drop: tokio_util::sync::DropGuard,
    task: tokio::task::JoinHandle<Result<(), Problem>>,
}

impl Runtime {
    pub fn start(
        role: ExitRole,
        plan: meshway_configuration::execution::ExitRolePlan,
    ) -> Result<Self, Problem> {
        let reader = role.reply_reader().map_err(|_| Problem::Underlay)?;
        let (sender, received) = mpsc::channel(CAPACITY);
        #[cfg(test)]
        testing::driver_started(sender.clone());
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

    pub fn packet(&mut self, secure: &mut SecureNode, packet: PendingAction) {
        let drained = self.drive(secure);
        // All operations created by this admission precede its packet in this
        // single FIFO. A full queue rejects the flow instead of waiting while
        // holding the SecureNode owner or dropping an unacknowledged callback.
        let flow = packet.exit_flow();
        if (!drained
            || self
                .sender
                .try_send(Work::Packet(Box::new(packet)))
                .is_err())
            && let Some(flow) = flow
        {
            secure.fail_exit_projection(&flow);
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
