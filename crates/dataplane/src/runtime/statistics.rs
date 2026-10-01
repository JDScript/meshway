use crate::{Error, TransportKind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tokio::sync::mpsc;

// A single definition keeps the fixed-size atomic storage and serialized fields aligned.
macro_rules! drop_reasons {
    ($($reason:ident => $field:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug)]
        pub(super) enum DropReason { $($reason),+ }

        #[derive(Clone, Copy, Debug, Default, serde::Serialize)]
        pub struct DropStatistics { $(pub $field: u64),+ }

        impl DropStatistics {
            pub fn total(&self) -> u64 {
                0_u64 $(.saturating_add(self.$field))+
            }
        }

        #[derive(Default)]
        struct DropCounters { $($field: AtomicU64),+ }

        impl DropCounters {
            fn record(&self, reason: DropReason) {
                match reason { $(DropReason::$reason => &self.$field),+ }
                    .fetch_add(1, Ordering::Relaxed);
            }

            fn snapshot(&self) -> DropStatistics {
                DropStatistics { $($field: self.$field.load(Ordering::Relaxed)),+ }
            }
        }
    };
}

drop_reasons! {
    IngressQueueFull => ingress_queue_full,
    IngressQueueClosed => ingress_queue_closed,
    OutboundQueueFull => outbound_queue_full,
    OutboundQueueClosed => outbound_queue_closed,
    EndpointDeliveryFull => endpoint_delivery_full,
    EndpointDeliveryClosed => endpoint_delivery_closed,
    Authentication => authentication,
    Replay => replay,
    Oversize => oversize,
    Protocol => protocol,
    Unavailable => unavailable,
    UnauthorizedPath => unauthorized_path,
    UnauthorizedCarrier => unauthorized_carrier,
    CapacityOther => capacity_other,
    StaleChannel => stale_channel,
    Io => io,
    Timeout => timeout,
    Closed => closed,
    Other => other,
}

impl From<Error> for DropReason {
    fn from(error: Error) -> Self {
        match error {
            Error::Authentication => Self::Authentication,
            Error::Replay => Self::Replay,
            Error::Oversize => Self::Oversize,
            Error::Protocol => Self::Protocol,
            Error::Unavailable => Self::Unavailable,
            Error::UnauthorizedPath => Self::UnauthorizedPath,
            Error::UnauthorizedTransport => Self::UnauthorizedCarrier,
            Error::Capacity => Self::CapacityOther,
            Error::Io => Self::Io,
            Error::Timeout => Self::Timeout,
            Error::Closed => Self::Closed,
            Error::InvalidConfig(_) => Self::Other,
        }
    }
}

/// Internal attribution travels to the owning attempt boundary; it is counted there once.
pub(super) struct Failure {
    pub error: Error,
    pub reason: DropReason,
    pub outgoing: Option<TransportKind>,
}

impl Failure {
    pub fn new(error: Error, reason: DropReason) -> Self {
        Self {
            error,
            reason,
            outgoing: None,
        }
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::new(error, error.into())
    }
}

pub(super) type StageResult<T> = std::result::Result<T, Failure>;

/// Queue entries, not bytes or frames. Occupancy and high-water are sampled observations.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct QueueStatistics {
    pub capacity: usize,
    pub occupancy: usize,
    pub high_water: usize,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct RuntimeQueueStatistics {
    pub ingress: QueueStatistics,
    pub endpoint_delivery: QueueStatistics,
}

/// Secondary outgoing-carrier attribution. Already included in drop_reasons elsewhere;
/// never add these values to dropped. A rejected batch is one enqueue attempt.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct QueueRejectionStatistics {
    pub full: u64,
    pub closed: u64,
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct TransportStatistics {
    pub received: u64,
    pub sent: u64,
    pub forwarded: u64,
    pub dropped: u64,
    pub drop_reasons: DropStatistics,
    pub outbound_queue_rejections: QueueRejectionStatistics,
    /// Maximum occupancy across currently selected queues of this carrier, not their sum.
    /// High-water retains the largest sample across this carrier's queues since startup.
    pub outbound_queue: QueueStatistics,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Statistics {
    pub native_udp: TransportStatistics,
    pub native_tcp: TransportStatistics,
    pub vless: TransportStatistics,
    pub queues: RuntimeQueueStatistics,
}

#[derive(Default)]
pub(super) struct HighWater(AtomicUsize);

impl HighWater {
    pub fn observe(&self, occupancy: usize) {
        if occupancy > self.0.load(Ordering::Relaxed) {
            self.0.fetch_max(occupancy, Ordering::Relaxed);
        }
    }

    pub fn enqueued<T>(&self, tx: &mpsc::Sender<T>) {
        // At least this entry was admitted, even if a concurrent consumer has drained it.
        self.observe(occupancy(tx).max(1));
    }

    pub fn snapshot(&self, capacity: usize, occupancy: usize) -> QueueStatistics {
        self.observe(occupancy);
        QueueStatistics {
            capacity,
            occupancy,
            high_water: self.0.load(Ordering::Relaxed),
        }
    }
}

pub(super) fn occupancy<T>(tx: &mpsc::Sender<T>) -> usize {
    tx.max_capacity() - tx.capacity()
}

#[derive(Default)]
pub(super) struct Counters {
    pub received: AtomicU64,
    pub sent: AtomicU64,
    pub forwarded: AtomicU64,
    drops: DropCounters,
    outbound_full: AtomicU64,
    outbound_closed: AtomicU64,
    pub outbound_high_water: HighWater,
}

impl Counters {
    pub fn drop(&self, reason: DropReason) {
        self.drops.record(reason);
    }

    pub fn outbound_rejected(&self, full: bool) {
        if full {
            &self.outbound_full
        } else {
            &self.outbound_closed
        }
        .fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self, capacity: usize, occupancy: usize) -> TransportStatistics {
        let drop_reasons = self.drops.snapshot();
        TransportStatistics {
            received: self.received.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            forwarded: self.forwarded.load(Ordering::Relaxed),
            // Sum this very snapshot, so concurrent increments cannot break the invariant.
            dropped: drop_reasons.total(),
            drop_reasons,
            outbound_queue_rejections: QueueRejectionStatistics {
                full: self.outbound_full.load(Ordering::Relaxed),
                closed: self.outbound_closed.load(Ordering::Relaxed),
            },
            outbound_queue: self.outbound_high_water.snapshot(capacity, occupancy),
        }
    }
}
