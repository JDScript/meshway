use super::credit::{Budget, Credit, STREAM_CAPACITY};
use super::{Error, Opened, PendingFrame, RecvJob, SendJob, open, seal};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    thread::JoinHandle,
};
use tokio::sync::{Notify, mpsc, oneshot};

pub enum Job {
    Send(SendJob),
    Receive(RecvJob),
}

pub enum Output {
    Send(PendingFrame),
    Receive(Opened),
}

impl Job {
    fn capacity(&self) -> usize {
        match self {
            Self::Send(job) => job.frame.body.capacity(),
            Self::Receive(job) => job.frame.body.capacity(),
        }
    }
    fn inline(&self) -> bool {
        // 52-byte TCP ACKs (and small IPv6/options variants) cost less than a
        // cross-thread handoff. Compare plaintext size on both endpoints;
        // receive bodies still contain the 16-byte Noise tag. This is only a
        // scheduling hint: authentication/admission and lane ordering are unchanged.
        match self {
            Self::Send(job) => job.frame.body.len() <= 128,
            Self::Receive(job) => job.frame.body.len() <= 128 + 16,
        }
    }
    fn epoch(&self) -> Option<Arc<()>> {
        match self {
            Self::Send(job) => job.guard.section.clone(),
            Self::Receive(job) => job.guard.section.clone(),
        }
    }
    fn key(&self) -> Key {
        let (frame, guard) = match self {
            Self::Send(job) => (&job.frame, &job.guard),
            Self::Receive(job) => (&job.frame, &job.guard),
        };
        Key(
            frame.context.circuit,
            frame.reply,
            guard
                .section
                .as_ref()
                .map_or(0, |epoch| Arc::as_ptr(epoch) as usize),
        )
    }
    pub fn circuit(&self) -> [u8; 32] {
        match self {
            Self::Send(job) => job.circuit(),
            Self::Receive(job) => job.circuit(),
        }
    }
    fn run(self) -> Result<Output, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("pipeline.worker");
        let _profile = PROFILE.enter();
        match self {
            Self::Send(job) => seal(job).map(Output::Send),
            Self::Receive(job) => open(job).map(Output::Receive),
        }
    }
}

struct Batch {
    jobs: Vec<(usize, Job)>,
    complete: oneshot::Sender<Vec<(usize, Result<Output, Error>)>>,
}

enum Work {
    Batch(Batch),
    Stream(Group),
}
type Group = Vec<(Key, u64, Job, Credit)>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key([u8; 32], bool, usize);

pub struct Ready {
    result: Result<Output, Error>,
    credit: Credit,
}
impl Ready {
    pub fn into_parts(self) -> (Result<Output, Error>, Credit) {
        (self.result, self.credit)
    }
}

type Completion = (Key, u64, Ready);

#[derive(Default)]
struct Lane {
    epoch: Option<Arc<()>>,
    submitted: u64,
    released: u64,
    completed: VecDeque<Option<Ready>>,
}

/// One bounded scheduling owner. Streaming admission retains at most 32 jobs,
/// including completed outputs and TX pending credits. Ready circuit/direction/epoch
/// prefixes are released round-robin in groups of at most 32, without waiting
/// for the rest of an input batch or any other circuit.
/// Zero workers executes exactly the same crypto inline, without a Tokio runtime.
pub struct Pool {
    senders: Vec<mpsc::Sender<Work>>,
    threads: Vec<JoinHandle<()>>,
    next: usize,
    completed: Arc<Mutex<VecDeque<Completion>>>,
    lanes: BTreeMap<Key, Lane>,
    queued: VecDeque<(Key, u64, Job, Credit)>,
    budget: Budget,
    ready_lanes: VecDeque<Key>,
    free_lanes: Vec<Lane>,
    groups: Arc<Mutex<Vec<Group>>>,
    dispatch_ready: Arc<Notify>,
}

impl Pool {
    pub fn default_workers() -> usize {
        // Hardware AEAD/SHA2 and a 32-packet window make handoff cost dominate
        // the measured 2/4 CPU owner. Explicit workers remain available for
        // other CPUs/workloads; never infer useful parallelism from quota alone.
        0
    }

    pub fn new(workers: usize) -> Result<Self, std::io::Error> {
        if workers > 256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker limit exceeded",
            ));
        }
        let completed = Arc::new(Mutex::new(VecDeque::with_capacity(STREAM_CAPACITY)));
        let mut pool = Self {
            senders: Vec::new(),
            threads: Vec::new(),
            next: 0,
            completed,
            lanes: BTreeMap::new(),
            queued: VecDeque::with_capacity(STREAM_CAPACITY),
            budget: Budget::default(),
            ready_lanes: VecDeque::with_capacity(STREAM_CAPACITY),
            free_lanes: (0..STREAM_CAPACITY)
                .map(|_| Lane {
                    completed: VecDeque::with_capacity(STREAM_CAPACITY),
                    ..Default::default()
                })
                .collect(),
            groups: Arc::new(Mutex::new(
                (0..STREAM_CAPACITY)
                    .map(|_| Vec::with_capacity(8))
                    .collect(),
            )),
            dispatch_ready: Arc::new(Notify::new()),
        };
        for index in 0..workers {
            let (send, mut receive) = mpsc::channel::<Work>(1);
            let complete = pool.completed.clone();
            let groups = pool.groups.clone();
            let dispatch_ready = pool.dispatch_ready.clone();
            let thread = std::thread::Builder::new()
                .name(format!("meshway-crypto-{index}"))
                .spawn(move || {
                    while let Some(work) = receive.blocking_recv() {
                        match work {
                            Work::Batch(work) => {
                                let results = work
                                    .jobs
                                    .into_iter()
                                    .map(|(index, job)| (index, job.run()))
                                    .collect();
                                let _ = work.complete.send(results);
                            }
                            Work::Stream(mut jobs) => {
                                for (circuit, sequence, job, mut credit) in jobs.drain(..) {
                                    if let Some(trace) = &mut credit.trace {
                                        trace.started();
                                    }
                                    // Every completion owns one of the globally bounded
                                    // credits; even an unpolled owner cannot fill this queue.
                                    let result = std::panic::catch_unwind(
                                        std::panic::AssertUnwindSafe(|| job.run()),
                                    )
                                    .unwrap_or(Err(Error::State));
                                    if let Some(trace) = &mut credit.trace {
                                        trace.ready();
                                    }
                                    let mut completed = complete.lock().expect("completion slots");
                                    let wake = completed.is_empty();
                                    completed.push_back((
                                        circuit,
                                        sequence,
                                        Ready { result, credit },
                                    ));
                                    drop(completed);
                                    if wake {
                                        dispatch_ready.notify_one();
                                    }
                                }
                                groups.lock().expect("descriptor pool").push(jobs);
                                dispatch_ready.notify_one();
                            }
                        }
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

    /// Check before owner admission/nonce consumption. Pressure on one circuit
    /// does not prevent another circuit from using the remaining global credits.
    pub fn can_submit(&self) -> bool {
        self.available() != 0
    }

    pub fn available(&self) -> usize {
        self.budget.available()
    }

    pub fn can_submit_capacity(&self, capacity: usize) -> bool {
        self.budget.can_reserve(capacity)
    }

    pub fn has_pending(&self) -> bool {
        !self.lanes.is_empty()
    }

    /// Never awaits a worker or a complete input batch. On pressure the caller
    /// retains the original admitted job, with its nonce and guard unchanged.
    pub fn submit(&mut self, job: Job) -> Option<Job> {
        let circuit = job.key();
        let Some(mut credit) = self.budget.reserve(job.capacity()) else {
            return Some(job);
        };
        let lane = self
            .lanes
            .entry(circuit)
            .or_insert_with(|| self.free_lanes.pop().expect("one lane per credit"));
        if lane.submitted == 0 {
            lane.epoch = job.epoch();
        }
        let sequence = lane.submitted;
        lane.submitted += 1;
        lane.completed.push_back(None);
        if self.senders.is_empty() || job.inline() {
            if let Some(trace) = &mut credit.trace {
                trace.started();
            }
            let result = job.run();
            if let Some(trace) = &mut credit.trace {
                trace.ready();
            }
            self.collect((circuit, sequence, Ready { result, credit }));
        } else {
            self.queued.push_back((circuit, sequence, job, credit));
        }
        None
    }

    /// Dispatch only ready work, with at most eight jobs per notification.
    pub fn flush(&mut self) {
        while !self.queued.is_empty() {
            let mut accepted = false;
            for _ in 0..self.senders.len() {
                let index = self.next;
                self.next = (self.next + 1) % self.senders.len();
                if let Ok(permit) = self.senders[index].try_reserve() {
                    let Some(mut jobs) = self.groups.lock().expect("descriptor pool").pop() else {
                        break;
                    };
                    jobs.extend(self.queued.drain(..self.queued.len().min(8)));
                    super::timing::batch("pipeline.dispatch.batch", jobs.len());
                    permit.send(Work::Stream(jobs));
                    accepted = true;
                    break;
                }
            }
            if !accepted {
                break;
            }
        }
    }

    fn collect(&mut self, (circuit, sequence, output): Completion) {
        let lane = self.lanes.get_mut(&circuit).expect("admitted lane");
        lane.completed[(sequence - lane.released) as usize] = Some(output);
        if sequence == lane.released {
            self.ready_lanes.push_back(circuit);
        }
    }

    /// Cancellation safe: notification does not release a credit or output.
    pub async fn ready(&mut self) {
        self.flush();
        if !self.ready_lanes.is_empty() {
            return;
        }
        if self.completed.lock().expect("completion slots").is_empty() {
            self.dispatch_ready.notified().await;
        }
        self.flush();
    }

    /// Release the ready per-circuit prefixes; unfinished work in another lane
    /// is never a barrier. No timer or minimum batch size delays ACK/small packets.
    pub fn take_ready(&mut self) -> Vec<Ready> {
        {
            let completed = self.completed.clone();
            let mut completed = completed.lock().expect("completion slots");
            while let Some(completion) = completed.pop_front() {
                self.collect(completion);
            }
        }
        self.flush();
        let mut outputs = Vec::new();
        while outputs.len() < 32 {
            let Some(circuit) = self.ready_lanes.pop_front() else {
                break;
            };
            let lane = self.lanes.get_mut(&circuit).expect("ready lane");
            while outputs.len() < 32 {
                if !lane.completed.front().is_some_and(Option::is_some) {
                    break;
                }
                let mut output = lane
                    .completed
                    .pop_front()
                    .flatten()
                    .expect("completed prefix");
                lane.released += 1;
                if let Some(trace) = &mut output.credit.trace {
                    trace.release();
                }
                outputs.push(output);
            }
            if lane.completed.front().is_some_and(Option::is_some) {
                self.ready_lanes.push_back(circuit);
            }
            if lane.released == lane.submitted {
                let mut lane = self.lanes.remove(&circuit).expect("completed lane");
                lane.submitted = 0;
                lane.released = 0;
                lane.epoch = None;
                self.free_lanes.push(lane);
            }
        }
        super::timing::batch("pipeline.release.batch", outputs.len());
        outputs
    }

    /// No wait to fill a batch. Queue pressure retains already-admitted jobs
    /// until the selected worker accepts them; it never drops an admitted suffix.
    pub async fn run(&mut self, jobs: &mut VecDeque<Job>) -> Vec<Result<Output, Error>> {
        self.run_prepared(jobs, Ok).await
    }

    /// Prepare small ready groups on the owner while earlier groups compute.
    /// The closure never leaves this calling task; only its opaque Job crosses
    /// the worker boundary. This overlaps owner admission even with one worker.
    pub async fn run_prepared<T>(
        &mut self,
        inputs: &mut VecDeque<T>,
        mut prepare: impl FnMut(T) -> Result<Job, Error>,
    ) -> Vec<Result<Output, Error>> {
        let count = inputs.len().min(32);
        if self.senders.is_empty() {
            return inputs
                .drain(..count)
                .map(|input| prepare(input).and_then(Job::run))
                .collect();
        }
        let mut receivers = Vec::new();
        let mut outputs: Vec<_> = (0..count).map(|_| Err(Error::State)).collect();
        let quantum = count.div_ceil(self.senders.len()).clamp(1, 8);
        for start in (0..count).step_by(quantum) {
            let end = (start + quantum).min(count);
            let mut jobs = Vec::with_capacity(end - start);
            for (index, output) in outputs.iter_mut().enumerate().take(end).skip(start) {
                match prepare(inputs.pop_front().expect("bounded input prefix")) {
                    Ok(job) => jobs.push((index, job)),
                    Err(error) => *output = Err(error),
                }
            }
            if jobs.is_empty() {
                continue;
            }
            let sender = &self.senders[self.next];
            self.next = (self.next + 1) % self.senders.len();
            let (complete, receive) = oneshot::channel();
            // One notification per ready worker group, never per packet. A
            // cancelled prior run can occupy at most one queued group/worker.
            let _ = sender.send(Work::Batch(Batch { jobs, complete })).await;
            receivers.push(receive);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(pool: &mut Pool, key: Key, ordinal: u64, error: Error, credit: Credit) {
        pool.collect((
            key,
            ordinal,
            Ready {
                result: Err(error),
                credit,
            },
        ));
    }

    #[tokio::test]
    async fn delayed_prefix_does_not_block_other_circuits_or_release_out_of_order() {
        let mut pool = Pool::new(0).unwrap();
        for id in [Key([1; 32], false, 1), Key([2; 32], false, 1)] {
            pool.lanes.insert(
                id,
                Lane {
                    epoch: None,
                    submitted: 2,
                    released: 0,
                    completed: VecDeque::from([None, None]),
                },
            );
        }
        let mut credits: Vec<_> = (0..4).map(|_| pool.budget.reserve(1280).unwrap()).collect();
        complete(
            &mut pool,
            Key([1; 32], false, 1),
            1,
            Error::Proof,
            credits.pop().unwrap(),
        );
        complete(
            &mut pool,
            Key([2; 32], false, 1),
            0,
            Error::Wire,
            credits.pop().unwrap(),
        );
        pool.ready().await;
        assert!(matches!(
            pool.take_ready().as_slice(),
            [Ready {
                result: Err(Error::Wire),
                ..
            }]
        ));
        assert_eq!(pool.available(), 29);
        complete(
            &mut pool,
            Key([1; 32], false, 1),
            0,
            Error::Crypto,
            credits.pop().unwrap(),
        );
        assert!(matches!(
            pool.take_ready().as_slice(),
            [
                Ready {
                    result: Err(Error::Crypto),
                    ..
                },
                Ready {
                    result: Err(Error::Proof),
                    ..
                }
            ]
        ));
        complete(
            &mut pool,
            Key([2; 32], false, 1),
            1,
            Error::Replay,
            credits.pop().unwrap(),
        );
        assert!(matches!(
            pool.take_ready().as_slice(),
            [Ready {
                result: Err(Error::Replay),
                ..
            }]
        ));
        assert!(!pool.has_pending());
        assert!(pool.lanes.is_empty());
    }

    #[test]
    fn directions_and_epochs_are_independent_and_tx_retains_credit() {
        let mut pool = Pool::new(0).unwrap();
        let keys = [
            Key([1; 32], false, 1),
            Key([1; 32], true, 1),
            Key([1; 32], false, 2),
        ];
        for id in keys {
            pool.lanes.insert(
                id,
                Lane {
                    epoch: None,
                    submitted: 1,
                    released: 0,
                    completed: VecDeque::from([None]),
                },
            );
        }
        let mut credits: Vec<_> = (0..3).map(|_| pool.budget.reserve(1280).unwrap()).collect();
        complete(&mut pool, keys[1], 0, Error::Proof, credits.pop().unwrap());
        complete(&mut pool, keys[2], 0, Error::Wire, credits.pop().unwrap());
        let tx = pool.take_ready();
        assert_eq!(tx.len(), 2);
        assert_eq!(pool.available(), 29, "release still owns TX credit");
        drop(tx);
        assert_eq!(pool.available(), 31);
        complete(&mut pool, keys[0], 0, Error::Crypto, credits.pop().unwrap());
        drop(pool.take_ready());
        assert_eq!(pool.available(), 32);
    }
}
