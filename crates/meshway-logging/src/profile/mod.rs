//! Opt-in, bounded process profiling. No task, subscriber or authority is owned
//! here. Executables initialize via logging and explicitly call report().
//! Synchronous scopes measure inclusive/exclusive monotonic wall and thread CPU;
//! future wrappers measure poll work, never the time suspended between polls.
mod stack;

use serde::Serialize;
use std::{
    collections::BTreeMap,
    future::{Future, poll_fn},
    sync::{
        Mutex, Once, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_METRICS: usize = 128;
static RATE: AtomicU64 = AtomicU64::new(0);
static DETAIL: AtomicU64 = AtomicU64::new(0);
static LOST: AtomicU64 = AtomicU64::new(0);
static METRICS: Mutex<Vec<&'static Metric>> = Mutex::new(Vec::new());
static EVENTS: Mutex<BTreeMap<(&'static str, &'static str), u64>> = Mutex::new(BTreeMap::new());
static GAUGES: Mutex<BTreeMap<&'static str, Option<u64>>> = Mutex::new(BTreeMap::new());

pub(super) fn configure() -> Result<(), crate::Error> {
    let rate = match std::env::var_os("MESHWAY_PROFILE") {
        None => 0,
        Some(value) => value
            .to_str()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|rate| *rate <= 65_536)
            .ok_or(crate::Error::Profile)?,
    };
    RATE.store(rate, Ordering::Relaxed);
    let detail = match std::env::var_os("MESHWAY_PROFILE_DETAIL") {
        None => 0,
        Some(value) => value
            .to_str()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|detail| *detail <= 2)
            .ok_or(crate::Error::Profile)?,
    };
    DETAIL.store(detail, Ordering::Relaxed);
    Ok(())
}

pub fn enabled() -> bool {
    RATE.load(Ordering::Relaxed) != 0
}

/// Static names only; no peer IDs, payloads or dynamic labels enter the registry.
pub struct Metric {
    name: &'static str,
    detail: u64,
    registered: Once,
    calls: AtomicU64,
    times: Mutex<Times>,
}

#[derive(Clone, Default, Serialize)]
struct Times {
    samples: u64,
    wall_ns: u64,
    self_wall_ns: u64,
    max_wall_ns: u64,
    cpu_samples: u64,
    self_cpu_samples: u64,
    cpu_ns: Option<u64>,
    self_cpu_ns: Option<u64>,
}

impl Metric {
    pub const fn new(name: &'static str) -> Self {
        Self::detail(name, 0)
    }

    /// High-frequency sublayers are separately enabled to bound observer bias.
    pub const fn detail(name: &'static str, level: u64) -> Self {
        Self {
            name,
            detail: level,
            registered: Once::new(),
            calls: AtomicU64::new(0),
            times: Mutex::new(Times {
                samples: 0,
                wall_ns: 0,
                self_wall_ns: 0,
                max_wall_ns: 0,
                cpu_samples: 0,
                self_cpu_samples: 0,
                cpu_ns: None,
                self_cpu_ns: None,
            }),
        }
    }

    fn begin(&'static self) -> u64 {
        let rate = RATE.load(Ordering::Relaxed);
        if rate != 0 && self.detail > DETAIL.load(Ordering::Relaxed) {
            return 0;
        }
        if rate != 0 {
            self.registered.call_once(|| {
                if let Ok(mut metrics) = METRICS.lock() {
                    if metrics.len() < MAX_METRICS && self.name.len() <= 96 {
                        metrics.push(self);
                    } else {
                        LOST.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        rate
    }

    pub fn enter(&'static self) -> stack::Scope {
        stack::enter(self, self.begin())
    }

    /// Wrap poll work only. Scope cannot escape poll or migrate between threads.
    pub async fn poll<F: Future>(&'static self, future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        poll_fn(|cx| {
            let _scope = self.enter();
            future.as_mut().poll(cx)
        })
        .await
    }

    /// An already-observed queue/lifetime duration; no thread CPU is inferred.
    pub fn elapsed(&'static self, duration: Duration) {
        if self.begin() != 0 {
            let ns = nanos(duration);
            self.record(ns, ns, None);
        }
    }

    fn record(&self, wall: u64, exclusive_wall: u64, cpu: Option<(u64, Option<u64>)>) {
        if let Ok(mut times) = self.times.lock() {
            times.samples = times.samples.saturating_add(1);
            times.wall_ns = times.wall_ns.saturating_add(wall);
            times.self_wall_ns = times.self_wall_ns.saturating_add(exclusive_wall);
            times.max_wall_ns = times.max_wall_ns.max(wall);
            if let Some((inclusive, exclusive)) = cpu {
                times.cpu_samples = times.cpu_samples.saturating_add(1);
                times.cpu_ns = Some(times.cpu_ns.unwrap_or(0).saturating_add(inclusive));
                if let Some(exclusive) = exclusive {
                    times.self_cpu_samples = times.self_cpu_samples.saturating_add(1);
                    times.self_cpu_ns =
                        Some(times.self_cpu_ns.unwrap_or(0).saturating_add(exclusive));
                }
            }
        }
    }
}

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// Exact cumulative diagnostic counter, bounded by static category/reason pairs.
pub fn event(category: &'static str, reason: &'static str) {
    if !enabled() {
        return;
    }
    if let Ok(mut events) = EVENTS.lock() {
        let key = (category, reason);
        if category.len() > 96
            || reason.len() > 96
            || (events.len() >= MAX_METRICS && !events.contains_key(&key))
        {
            LOST.fetch_add(1, Ordering::Relaxed);
        } else {
            let count = events.entry(key).or_default();
            *count = count.saturating_add(1);
        }
    }
}

/// Latest owner-observed gauge. None explicitly denotes unavailable ownership.
pub fn gauge(name: &'static str, value: Option<u64>) {
    if !enabled() {
        return;
    }
    if let Ok(mut gauges) = GAUGES.lock() {
        if name.len() <= 96 && (gauges.len() < MAX_METRICS || gauges.contains_key(name)) {
            gauges.insert(name, value);
        } else {
            LOST.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Owner-driven and paced; emits cumulative diagnostic observations on stderr
/// through the executable's normal tracing subscriber. No history/State sample.
pub fn report() {
    if !enabled() {
        return;
    }
    static LAST: OnceLock<Mutex<(Instant, Instant)>> = OnceLock::new();
    let now = Instant::now();
    let Ok(mut last) = LAST.get_or_init(|| Mutex::new((now, now))).try_lock() else {
        return;
    };
    if now.duration_since(last.1) < Duration::from_secs(5) {
        return;
    }
    last.1 = now;
    let elapsed = nanos(now.duration_since(last.0));
    drop(last);
    #[derive(Serialize)]
    struct View {
        name: &'static str,
        calls_started: u64,
        times: Times,
    }
    let views = METRICS
        .lock()
        .map(|metrics| {
            metrics
                .iter()
                .filter_map(|metric| {
                    let times = metric.times.lock().ok()?;
                    let calls = metric.calls.load(Ordering::Relaxed);
                    (calls != 0).then(|| View {
                        name: metric.name,
                        calls_started: calls,
                        times: times.clone(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let events = EVENTS
        .lock()
        .map(|events| {
            events
                .iter()
                .map(|((category, reason), count)| (*category, *reason, *count))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let gauges = GAUGES
        .lock()
        .map(|gauges| gauges.clone())
        .unwrap_or_default();
    if let (Ok(metrics), Ok(events), Ok(gauges)) = (
        serde_json::to_string(&views),
        serde_json::to_string(&events),
        serde_json::to_string(&gauges),
    ) {
        tracing::info!(target: "meshway_profile", profile_version = 1, elapsed_ns = elapsed,
            sample_denominator = RATE.load(Ordering::Relaxed), lost = LOST.load(Ordering::Relaxed),
            detail = DETAIL.load(Ordering::Relaxed),
            metrics_json = %metrics, counters_json = %events, gauges_json = %gauges, "Sampled packet profile");
    }
}
