//! Observer-only wall clocks. Sample one in 128 admissions per calling thread;
//! disabled profiling performs no clock read or allocation.
use meshway_logging::profile::{self, Metric};
use std::{cell::Cell, time::Instant};

pub(crate) fn start() -> Option<Instant> {
    thread_local! { static NEXT: Cell<u64> = const { Cell::new(0) }; }
    if !profile::enabled() {
        return None;
    }
    NEXT.with(|next| {
        let value = next.get();
        next.set(value.wrapping_add(1));
        value.is_multiple_of(128).then(Instant::now)
    })
}

pub(crate) fn batch(category: &'static str, count: usize) {
    if count != 0 {
        profile::event(
            category,
            match count {
                1 => "1",
                2..=4 => "2-4",
                5..=8 => "5-8",
                9..=16 => "9-16",
                _ => "17-32",
            },
        );
    }
}

pub(super) struct Trace {
    at: Instant,
    worker_at: Option<Instant>,
    released: bool,
}

impl Trace {
    pub fn new() -> Option<Box<Self>> {
        start().map(|at| {
            Box::new(Self {
                at,
                worker_at: None,
                released: false,
            })
        })
    }
    pub fn started(&mut self) {
        static WALL: Metric = Metric::new("pipeline.submit_to_worker.wall");
        let now = Instant::now();
        WALL.elapsed(now.duration_since(self.at));
        self.worker_at = Some(now);
    }
    pub fn ready(&mut self) {
        static WALL: Metric = Metric::new("pipeline.submit_to_ready.wall");
        let now = Instant::now();
        WALL.elapsed(now.duration_since(self.at));
        if let Some(at) = self.worker_at.take() {
            static WORK: Metric = Metric::new("pipeline.worker.wall");
            WORK.elapsed(now.duration_since(at));
        }
        self.at = now;
    }
    pub fn release(&mut self) {
        static WALL: Metric = Metric::new("pipeline.ready_to_release.wall");
        let now = Instant::now();
        WALL.elapsed(now.duration_since(self.at));
        self.at = now;
        self.released = true;
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if self.released {
            static WALL: Metric = Metric::new("pipeline.release_to_credit_drop.wall");
            WALL.elapsed(self.at.elapsed());
        }
    }
}
