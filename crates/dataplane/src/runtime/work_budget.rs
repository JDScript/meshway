/// Task-local frame accounting, independent of Tokio's I/O-operation budget.
/// Call only after a complete frame attempt, outside select branch futures and
/// after releasing synchronous locks. No task, timer, or boxed future per frame.
pub(super) struct WorkBudget {
    limit: usize,
    remaining: usize,
}

impl WorkBudget {
    pub fn new(frames: usize) -> Self {
        Self {
            limit: frames,
            remaining: frames,
        }
    }

    pub async fn completed_frame(&mut self) {
        if self.limit == 0 {
            return;
        }
        self.remaining -= 1;
        if self.remaining == 0 {
            self.remaining = self.limit;
            tokio::task::yield_now().await;
        }
    }
}
