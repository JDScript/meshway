use super::{LOST, Metric, nanos};
use std::{
    cell::RefCell,
    hash::{Hash, Hasher},
    marker::PhantomData,
    rc::Rc,
    sync::atomic::Ordering,
    time::{Instant, SystemTime},
};

const MAX_DEPTH: usize = 64;
struct Timing {
    wall: Instant,
    cpu: Option<u64>,
    child_wall: u64,
    child_cpu: Option<u64>,
}
struct Frame {
    metric: &'static Metric,
    timing: Option<Timing>,
}
struct Stack {
    frames: Vec<Frame>,
    random: u64,
}
impl Default for Stack {
    fn default() -> Self {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::thread::current().id().hash(&mut hash);
        SystemTime::now().hash(&mut hash);
        Self {
            frames: Vec::with_capacity(MAX_DEPTH),
            random: hash.finish().max(1),
        }
    }
}
impl Stack {
    fn sampled(&mut self, rate: u64) -> bool {
        // Profiling selection only, never cryptographic randomness. Randomized
        // roots avoid aliasing periodic send/receive event patterns at rate N.
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        self.random.is_multiple_of(rate)
    }
}
thread_local! { static STACK: RefCell<Stack> = RefCell::new(Stack::default()); }

/// A synchronous, thread-bound scope. Holding it across await makes that future
/// non-Send; async callers use Metric::poll instead.
pub struct Scope {
    metric: Option<&'static Metric>,
    _thread: PhantomData<Rc<()>>,
}

pub(super) fn enter(metric: &'static Metric, rate: u64) -> Scope {
    let mut scope = Scope {
        metric: None,
        _thread: PhantomData,
    };
    if rate == 0 {
        return scope;
    }
    STACK.with_borrow_mut(|stack| {
        let sampled = match stack.frames.last() {
            Some(frame) if frame.timing.is_none() => return,
            Some(_) => true,
            None => stack.sampled(rate),
        };
        if stack.frames.len() >= MAX_DEPTH {
            LOST.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let timing = sampled.then(|| Timing {
            wall: Instant::now(),
            cpu: cpu_time(),
            child_wall: 0,
            child_cpu: Some(0),
        });
        stack.frames.push(Frame { metric, timing });
        scope.metric = Some(metric);
    });
    scope
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some(metric) = self.metric else {
            return;
        };
        STACK.with_borrow_mut(|stack| {
            let Some(frame) = stack.frames.pop() else {
                LOST.fetch_add(1, Ordering::Relaxed);
                return;
            };
            if !std::ptr::eq(metric, frame.metric) {
                stack.frames.clear();
                LOST.fetch_add(1, Ordering::Relaxed);
                return;
            }
            if let Some(timing) = frame.timing {
                let cpu = timing
                    .cpu
                    .zip(cpu_time())
                    .and_then(|(start, end)| end.checked_sub(start));
                let wall = nanos(timing.wall.elapsed());
                metric.record(
                    wall,
                    wall.saturating_sub(timing.child_wall),
                    cpu.map(|cpu| (cpu, timing.child_cpu.map(|child| cpu.saturating_sub(child)))),
                );
                if let Some(parent) = stack
                    .frames
                    .last_mut()
                    .and_then(|frame| frame.timing.as_mut())
                {
                    parent.child_wall = parent.child_wall.saturating_add(wall);
                    parent.child_cpu = parent
                        .child_cpu
                        .zip(cpu)
                        .map(|(parent, child)| parent.saturating_add(child));
                }
            }
        });
    }
}

#[cfg(unix)]
fn cpu_time() -> Option<u64> {
    let time = nix::time::clock_gettime(nix::time::ClockId::CLOCK_THREAD_CPUTIME_ID).ok()?;
    u64::try_from(time.tv_sec())
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.tv_nsec()).ok()?)
}
#[cfg(not(unix))]
fn cpu_time() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_scopes_account_children_once_and_unsampled_roots_suppress_children() {
        static PARENT: Metric = Metric::new("test.parent");
        static CHILD: Metric = Metric::new("test.child");
        let parent = enter(&PARENT, 1);
        let child = enter(&CHILD, 1);
        std::hint::black_box(SystemTime::now());
        drop(child);
        drop(parent);
        let parent = PARENT.times.lock().unwrap().clone();
        let child = CHILD.times.lock().unwrap().clone();
        assert_eq!(parent.samples, 1);
        assert_eq!(child.samples, 1);
        assert_eq!(parent.self_wall_ns + child.wall_ns, parent.wall_ns);
        if let (Some(parent_cpu), Some(self_cpu), Some(child_cpu)) =
            (parent.cpu_ns, parent.self_cpu_ns, child.cpu_ns)
        {
            assert_eq!(self_cpu + child_cpu, parent_cpu);
        }
        STACK.with_borrow_mut(|stack| {
            stack.frames.push(Frame {
                metric: &PARENT,
                timing: None,
            })
        });
        drop(enter(&CHILD, 1));
        STACK.with_borrow_mut(|stack| {
            stack.frames.pop();
        });
        assert_eq!(CHILD.times.lock().unwrap().samples, 1);
        assert!(STACK.with_borrow(|stack| stack.frames.is_empty()));
    }
}
