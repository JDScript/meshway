use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const STREAM_CAPACITY: usize = 32;
/// A normal MTU packet including all reserved storage, not just live payload.
pub const MTU_STORAGE: usize = 4096;
/// Oversized frames have a separate reservation class; never 32 jumbo slots.
pub const JUMBO_CAPACITY: usize = 2;
pub const JUMBO_STORAGE: usize =
    crate::secure::MAX_FRAME + crate::DEFAULT_HEADROOM + crate::DEFAULT_TAILROOM;
pub const STREAM_BYTES: usize =
    (STREAM_CAPACITY - JUMBO_CAPACITY) * MTU_STORAGE + JUMBO_CAPACITY * JUMBO_STORAGE;

#[derive(Default)]
struct Usage {
    packets: AtomicUsize,
    bytes: AtomicUsize,
    jumbo: AtomicUsize,
}

#[derive(Default)]
pub(super) struct Budget(Arc<Usage>);

impl Budget {
    pub fn available(&self) -> usize {
        STREAM_CAPACITY - self.0.packets.load(Ordering::Acquire)
    }

    pub fn can_reserve(&self, capacity: usize) -> bool {
        capacity <= JUMBO_STORAGE
            && self.available() != 0
            && self.0.bytes.load(Ordering::Acquire) + capacity <= STREAM_BYTES
            && (capacity <= MTU_STORAGE || self.0.jumbo.load(Ordering::Acquire) < JUMBO_CAPACITY)
    }

    /// Only the unique Pool owner reserves; any final output owner can release.
    pub fn reserve(&mut self, capacity: usize) -> Option<Credit> {
        if !self.can_reserve(capacity) {
            return None;
        }
        self.0.packets.fetch_add(1, Ordering::AcqRel);
        self.0.bytes.fetch_add(capacity, Ordering::AcqRel);
        if capacity > MTU_STORAGE {
            self.0.jumbo.fetch_add(1, Ordering::AcqRel);
        }
        Some(Credit {
            usage: self.0.clone(),
            capacity,
            trace: super::timing::Trace::new(),
        })
    }
}

/// Retained through final TX/TUN completion or explicit cancellation/failure.
pub struct Credit {
    usage: Arc<Usage>,
    capacity: usize,
    pub(super) trace: Option<Box<super::timing::Trace>>,
}

impl Credit {
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Drop for Credit {
    fn drop(&mut self) {
        self.usage.bytes.fetch_sub(self.capacity, Ordering::Release);
        if self.capacity > MTU_STORAGE {
            self.usage.jumbo.fetch_sub(1, Ordering::Release);
        }
        self.usage.packets.fetch_sub(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtu_and_jumbo_reservations_bound_bytes_until_final_owner_releases() {
        let mut budget = Budget::default();
        let mut jumbo: Vec<_> = (0..JUMBO_CAPACITY)
            .map(|_| budget.reserve(JUMBO_STORAGE).unwrap())
            .collect();
        assert!(!budget.can_reserve(JUMBO_STORAGE));
        let small: Vec<_> = (0..STREAM_CAPACITY - JUMBO_CAPACITY)
            .map(|_| budget.reserve(MTU_STORAGE).unwrap())
            .collect();
        assert_eq!(budget.available(), 0);
        assert_eq!(budget.0.bytes.load(Ordering::Acquire), STREAM_BYTES);
        drop(jumbo.pop());
        assert!(budget.can_reserve(JUMBO_STORAGE));
        assert_eq!(budget.available(), 1);
        drop((small, jumbo));
        assert_eq!(budget.available(), STREAM_CAPACITY);
        assert_eq!(budget.0.bytes.load(Ordering::Acquire), 0);
        assert!(!budget.can_reserve(JUMBO_STORAGE + 1));
    }

    #[tokio::test]
    async fn stalled_tx_retains_credit_across_cancelled_waits_and_releases_fifo() {
        use crate::carrier::SendPending;
        let mut budget = Budget::default();
        let mut pending = SendPending::default();
        let (send, receive) = tokio::sync::oneshot::channel();
        pending.push(vec![budget.reserve(MTU_STORAGE).unwrap()], async move {
            receive.await.unwrap()
        });
        for n in 1..STREAM_CAPACITY {
            pending.push(vec![budget.reserve(MTU_STORAGE).unwrap()], async move { n });
        }
        assert_eq!(budget.available(), 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), pending.ready())
                .await
                .is_err()
        );
        assert_eq!(
            budget.available(),
            0,
            "cancelled waiter must not free TX storage"
        );
        send.send(0).unwrap();
        for n in 0..STREAM_CAPACITY {
            assert_eq!(pending.ready().await, n);
            assert_eq!(budget.available(), n + 1);
        }
        assert!(pending.is_empty());
        pending.push(
            vec![budget.reserve(JUMBO_STORAGE).unwrap()],
            std::future::pending::<usize>(),
        );
        drop(pending);
        assert_eq!(
            budget.available(),
            STREAM_CAPACITY,
            "explicit cancellation frees credits"
        );
    }
}
