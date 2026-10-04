//! A bounded commit journal and SSE transport. The provider performs each poll
//! on the same serial owner as commits, reauthorizing before returning frames;
//! between polls a stream waits for a wake-up, never on a timer.
use crate::effect::Effect;
use crate::error::Error;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{Stream, stream};
use meshway_model::{NetworkId, StateEpoch};
use serde::Serialize;
use std::{
    collections::VecDeque, convert::Infallible, future::Future, pin::Pin, sync::Arc, time::Duration,
};

pub const MAX_REPLAY_EVENTS: usize = 512;
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct JournalEntry {
    pub sequence: u64,
    pub change: Effect,
}
pub struct EventLog {
    pub epoch: StateEpoch,
    sequence: u64,
    entries: VecDeque<JournalEntry>,
}
impl EventLog {
    pub fn new(epoch: StateEpoch) -> Self {
        Self {
            epoch,
            sequence: 0,
            entries: VecDeque::new(),
        }
    }
    pub fn append(&mut self, change: Effect) {
        // Sequence exhaustion ends replay in this epoch instead of wrapping IDs.
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.entries.clear();
            return;
        };
        self.sequence = sequence;
        self.entries.push_back(JournalEntry { sequence, change });
        while self.entries.len() > MAX_REPLAY_EVENTS {
            self.entries.pop_front();
        }
    }
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    pub fn after(&self, sequence: u64, network: NetworkId) -> Result<Vec<JournalEntry>, Error> {
        if sequence > self.sequence
            || self
                .entries
                .front()
                .is_some_and(|first| sequence.saturating_add(1) < first.sequence)
        {
            return Err(Error::InvalidCursor);
        }
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.sequence > sequence && entry.change.network() == Some(network))
            .cloned()
            .collect())
    }
}

#[derive(Clone)]
pub struct SseFrame {
    pub id: Option<String>,
    pub data: String,
}
impl SseFrame {
    pub fn encode<T: Serialize>(id: Option<String>, value: &T) -> Result<Self, Error> {
        let data = serde_json::to_string(value).map_err(|_| Error::encode("sse frame"))?;
        if data.len() > MAX_FRAME_BYTES
            || id
                .as_ref()
                .is_some_and(|id| id.len() > 4096 || id.contains(['\n', '\r', '\0']))
        {
            return Err(Error::StateLimit);
        }
        Ok(Self { id, data })
    }
}
pub enum StreamPoll {
    Frame(SseFrame),
    Pending,
    Closed,
}
pub type PollFuture<'a> = Pin<Box<dyn Future<Output = Result<StreamPoll, Error>> + Send + 'a>>;
pub type WaitFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// One call produces at most one frame and must recheck current authorization,
/// even when draining an initial snapshot or replay buffer. Pending must not
/// extend any authentication, execution or observation lease.
pub trait StreamSource: Send + Sync + 'static {
    fn poll<'a>(&'a self, subscription: &'a str) -> PollFuture<'a>;
    /// Resolves when a Pending subscription should poll again: a committed
    /// change it may see, or a deadline it computed. Returns at once for an
    /// unknown subscription.
    fn wait<'a>(&'a self, subscription: &'a str) -> WaitFuture<'a>;
    fn cancel(&self, subscription: &str);
}
struct StreamGuard {
    source: Arc<dyn StreamSource>,
    subscription: String,
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.source.cancel(&self.subscription);
    }
}

pub fn response(
    source: Arc<dyn StreamSource>,
    subscription: String,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let guard = StreamGuard {
        source,
        subscription,
    };
    let output = stream::unfold(Some(guard), |guard| async move {
        let guard = guard?;
        loop {
            match guard.source.poll(&guard.subscription).await {
                Ok(StreamPoll::Frame(frame)) => {
                    let mut event = Event::default().data(frame.data);
                    if let Some(id) = frame.id {
                        event = event.id(id);
                    }
                    return Some((Ok(event), Some(guard)));
                }
                Ok(StreamPoll::Pending) => guard.source.wait(&guard.subscription).await,
                Ok(StreamPoll::Closed) | Err(_) => return None,
            }
        }
    });
    Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keepalive"),
    )
}

/// Retained per-view frame buffer. Replay IDs are cryptographically random,
/// bound to this authorized subscription by the enclosing service. A cursor
/// alone never creates or authenticates a view.
#[derive(Default)]
pub struct ReplayBuffer {
    frames: VecDeque<SseFrame>,
    bytes: usize,
}
impl ReplayBuffer {
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    pub fn push(&mut self, frame: SseFrame) {
        if frame.id.is_some() && self.frames.iter().any(|old| old.id == frame.id) {
            return;
        }
        self.bytes += frame.data.len();
        self.frames.push_back(frame);
        while self.frames.len() > MAX_REPLAY_EVENTS || self.bytes > MAX_BATCH_BYTES {
            if let Some(frame) = self.frames.pop_front() {
                self.bytes -= frame.data.len();
            }
        }
    }
    pub fn after(&self, cursor: &str) -> Result<Vec<SseFrame>, Error> {
        let index = self
            .frames
            .iter()
            .position(|frame| frame.id.as_deref() == Some(cursor))
            .ok_or(Error::InvalidCursor)?;
        Ok(self.frames.iter().skip(index + 1).cloned().collect())
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
    }
}
