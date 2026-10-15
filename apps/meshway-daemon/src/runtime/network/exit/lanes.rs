//! Which way each exit flow's packets take to the exit TUN: through the
//! driver FIFO (first packets, packets waiting for an ACK) or straight to
//! the I/O owner's batches. A flow writes directly only after the driver
//! released one of its packets and while the FIFO holds none of them, so
//! the two ways never reorder one flow. Scheduling only: every packet keeps
//! its own admission, projection and guard.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Lane {
    /// The driver released a packet of the flow and asked for its
    /// conntrack confirmation; later packets need nothing more from it.
    direct: bool,
    /// The flow's packets in the driver FIFO or held by it.
    queued: u32,
}

/// Bounded by the flows the driver holds a packet of plus those it opened.
pub(super) struct Lanes<K>(Arc<Mutex<BTreeMap<K, Lane>>>);

impl<K> Clone for Lanes<K> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<K> Default for Lanes<K> {
    fn default() -> Self {
        Self(Arc::default())
    }
}

/// One packet's place in the driver FIFO; dropping it, however the driver
/// ends with the packet, frees the place.
pub(super) struct Queued<K: Ord> {
    lanes: Lanes<K>,
    flow: K,
}

impl<K: Ord> Drop for Queued<K> {
    fn drop(&mut self) {
        if let Ok(mut lanes) = self.lanes.0.lock()
            && let Some(lane) = lanes.get_mut(&self.flow)
        {
            lane.queued = lane.queued.saturating_sub(1);
            if lane.queued == 0 && !lane.direct {
                lanes.remove(&self.flow);
            }
        }
    }
}

impl<K: Ord + Copy> Lanes<K> {
    /// Whether `flow`'s next packet may skip the driver.
    pub fn direct(&self, flow: &K) -> bool {
        self.0.lock().is_ok_and(|lanes| {
            lanes
                .get(flow)
                .is_some_and(|lane| lane.direct && lane.queued == 0)
        })
    }

    /// Account a packet of `flow` entering the driver FIFO.
    pub fn enqueue(&self, flow: K) -> Option<Queued<K>> {
        let mut lanes = self.0.lock().ok()?;
        lanes.entry(flow).or_default().queued += 1;
        Some(Queued {
            lanes: self.clone(),
            flow,
        })
    }

    /// The driver released a packet of `flow` (or ended it): later packets
    /// write directly (or no longer do).
    pub fn set(&self, flow: K, direct: bool) {
        if let Ok(mut lanes) = self.0.lock() {
            if direct {
                lanes.entry(flow).or_default().direct = true;
            } else if let Some(lane) = lanes.get_mut(&flow) {
                lane.direct = false;
                if lane.queued == 0 {
                    lanes.remove(&flow);
                }
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.lock().map_or(0, |lanes| lanes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(byte: u8) -> u8 {
        byte
    }

    #[test]
    fn a_flow_writes_directly_only_with_nothing_of_it_in_the_driver() {
        let lanes = Lanes::<u8>::default();
        let first = lanes.enqueue(flow(1));
        assert!(!lanes.direct(&flow(1)));
        // Released: but a second packet is still queued behind it.
        let second = lanes.enqueue(flow(1));
        lanes.set(flow(1), true);
        drop(first);
        assert!(!lanes.direct(&flow(1)), "a queued packet keeps the order");
        drop(second);
        assert!(lanes.direct(&flow(1)));
        // A packet waiting for a renewal ACK takes the FIFO again.
        let renewing = lanes.enqueue(flow(1));
        assert!(!lanes.direct(&flow(1)));
        drop(renewing);
        assert!(lanes.direct(&flow(1)));
        assert!(!lanes.direct(&flow(2)));
        // Ended: nothing remains of it.
        lanes.set(flow(1), false);
        assert!(!lanes.direct(&flow(1)));
        assert_eq!(lanes.len(), 0);
        let late = lanes.enqueue(flow(3));
        lanes.set(flow(3), false);
        assert_eq!(lanes.len(), 1);
        drop(late);
        assert_eq!(lanes.len(), 0);
    }
}
