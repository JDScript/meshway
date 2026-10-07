use super::{Error, Opened, PendingFrame, RecvJob, SendJob, open, seal};
use std::{collections::VecDeque, thread::JoinHandle};
use tokio::sync::{mpsc, oneshot};

pub enum Job {
    Send(SendJob),
    Receive(RecvJob),
}

pub enum Output {
    Send(PendingFrame),
    Receive(Opened),
}

impl Job {
    fn run(self) -> Result<Output, Error> {
        match self {
            Self::Send(job) => seal(job).map(Output::Send),
            Self::Receive(job) => open(job).map(Output::Receive),
        }
    }
}

struct Work {
    jobs: Vec<(usize, Job)>,
    complete: oneshot::Sender<Vec<(usize, Result<Output, Error>)>>,
}

/// One bounded scheduling owner. Each call drains at most 32 jobs; all buffers,
/// including completed-but-uncollected outputs, remain within that budget.
/// Work is spread round-robin, including packets of one circuit. Results are returned in input order
/// (also across circuits), so completion races cannot reorder replay commits or
/// output. The caller must submit each returned group before driving the next.
/// Zero workers executes exactly the same crypto inline, without a Tokio runtime.
pub struct Pool {
    senders: Vec<mpsc::Sender<Work>>,
    threads: Vec<JoinHandle<()>>,
    next: usize,
}

impl Pool {
    pub fn default_workers() -> usize {
        std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1).max(1))
    }

    pub fn new(workers: usize) -> Result<Self, std::io::Error> {
        if workers > 256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker limit exceeded",
            ));
        }
        let mut pool = Self {
            senders: Vec::new(),
            threads: Vec::new(),
            next: 0,
        };
        for index in 0..workers {
            let (send, mut receive) = mpsc::channel::<Work>(1);
            let thread = std::thread::Builder::new()
                .name(format!("meshway-crypto-{index}"))
                .spawn(move || {
                    while let Some(work) = receive.blocking_recv() {
                        let results = work
                            .jobs
                            .into_iter()
                            .map(|(index, job)| (index, job.run()))
                            .collect();
                        let _ = work.complete.send(results);
                    }
                })?;
            pool.senders.push(send);
            pool.threads.push(thread);
        }
        Ok(pool)
    }

    pub fn workers(&self) -> usize {
        self.threads.len()
    }

    /// No wait to fill a batch. Queue pressure retains already-admitted jobs
    /// until the selected worker accepts them; it never drops an admitted suffix.
    pub async fn run(&mut self, jobs: &mut VecDeque<Job>) -> Vec<Result<Output, Error>> {
        let count = jobs.len().min(32);
        if self.senders.is_empty() {
            return jobs.drain(..count).map(Job::run).collect();
        }
        let mut groups: Vec<Vec<_>> = (0..self.senders.len()).map(|_| Vec::new()).collect();
        for (index, job) in jobs.drain(..count).enumerate() {
            let shard = self.next;
            self.next = (self.next + 1) % self.senders.len();
            groups[shard].push((index, job));
        }
        let mut receivers = Vec::new();
        for (sender, jobs) in self.senders.iter().zip(groups) {
            if jobs.is_empty() {
                continue;
            }
            let (complete, receive) = oneshot::channel();
            // One notification per ready worker group, never per packet. A
            // cancelled prior run can occupy at most one queued group/worker.
            let _ = sender.send(Work { jobs, complete }).await;
            receivers.push(receive);
        }
        let mut outputs: Vec<_> = (0..count).map(|_| Err(Error::State)).collect();
        for receive in receivers {
            if let Ok(results) = receive.await {
                for (index, result) in results {
                    outputs[index] = result;
                }
            }
        }
        outputs
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.senders.clear();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
