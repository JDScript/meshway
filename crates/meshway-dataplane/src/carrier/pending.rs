//! Bounded output drivers. Futures stay owned here across owner select turns;
//! they are never spawned per packet. Each direction has one FIFO and at most
//! one active batch, so completion order cannot reorder circuit output.
use super::{Error, batch::Delivery};
use crate::{PendingAction, secure::pipeline::Credit};
use std::{collections::VecDeque, future::Future, pin::Pin};

struct Entry<T> {
    value: T,
    credits: Vec<Credit>,
}

struct Queue<T> {
    entries: VecDeque<Entry<T>>,
    packets: usize,
    bytes: usize,
    jumbo: usize,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            packets: 0,
            bytes: 0,
            jumbo: 0,
        }
    }
}

impl<T> Queue<T> {
    fn push(&mut self, value: T, credits: Vec<Credit>) {
        use crate::secure::pipeline::{JUMBO_CAPACITY, MTU_STORAGE, STREAM_BYTES, STREAM_CAPACITY};
        let bytes: usize = credits.iter().map(Credit::capacity).sum();
        let jumbo = credits
            .iter()
            .filter(|credit| credit.capacity() > MTU_STORAGE)
            .count();
        // All entries must carry credits from the same globally bounded owner.
        // No output is admitted here without its existing lifecycle reservation.
        assert!(!credits.is_empty());
        assert!(self.packets + credits.len() <= STREAM_CAPACITY);
        assert!(self.bytes + bytes <= STREAM_BYTES);
        assert!(self.jumbo + jumbo <= JUMBO_CAPACITY);
        self.packets += credits.len();
        self.bytes += bytes;
        self.jumbo += jumbo;
        self.entries.push_back(Entry { value, credits });
    }

    fn pop(&mut self) -> T {
        let entry = self.entries.pop_front().expect("completed output");
        self.packets -= entry.credits.len();
        self.bytes -= entry.credits.iter().map(Credit::capacity).sum::<usize>();
        self.jumbo -= entry
            .credits
            .iter()
            .filter(|credit| credit.capacity() > crate::secure::pipeline::MTU_STORAGE)
            .count();
        entry.value
    }
}

type SendFuture<R> = Pin<Box<dyn Future<Output = R> + Send>>;

/// A single business TX FIFO. Construct each future before enqueueing to freeze
/// its channel selection; it must retain the original guards at every I/O poll.
pub struct SendPending<R>(Queue<SendFuture<R>>);

impl<R> Default for SendPending<R> {
    fn default() -> Self {
        Self(Queue::default())
    }
}

impl<R> SendPending<R> {
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }

    pub fn push(&mut self, credits: Vec<Credit>, send: impl Future<Output = R> + Send + 'static) {
        self.0.push(Box::pin(send), credits);
    }

    /// Cancelling this wait retains the exact future and all its credits.
    pub async fn ready(&mut self) -> R {
        let Some(entry) = self.0.entries.front_mut() else {
            return std::future::pending().await;
        };
        let result = entry.value.as_mut().await;
        drop(self.0.pop());
        result
    }
}

/// One bounded TUN delivery FIFO, independent of carrier TX readiness.
pub struct TunPending<M>(Queue<(Delivery, M)>);

impl<M> Default for TunPending<M> {
    fn default() -> Self {
        Self(Queue::default())
    }
}

impl<M> TunPending<M> {
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }

    pub fn push(&mut self, credits: Vec<Credit>, packets: Vec<PendingAction>, metadata: M) {
        assert_eq!(credits.len(), packets.len());
        self.0.push((Delivery::new(packets), metadata), credits);
    }

    /// Every successful syscall commits its prefix before the next await.
    /// A cancelled wait leaves the unwritten suffix and original lifetime here.
    pub async fn ready(&mut self, tun: &meshway_tun::Tun) -> (Result<(), Error>, M) {
        let Some(entry) = self.0.entries.front_mut() else {
            return std::future::pending().await;
        };
        let result = entry.value.0.write(tun).await;
        (result, self.0.pop().1)
    }
}
