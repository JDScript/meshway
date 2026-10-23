//! Unadmitted ingress has its own packet and storage bounds. These permits end
//! at dequeue, before the SecureNode owner reserves its independent credits.
use super::Event;
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

#[cfg(test)]
mod tests;

struct Queued {
    events: VecDeque<Event>,
    _frames: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

#[derive(Clone)]
pub(super) struct Sender {
    inner: mpsc::Sender<Queued>,
    bytes: Arc<Semaphore>,
    frames: Arc<Semaphore>,
    limit: usize,
}

pub(super) struct Receiver {
    inner: mpsc::Receiver<Queued>,
    pending: Option<Queued>,
    frames: Arc<Semaphore>,
    count: usize,
    observer: Option<Arc<WakeObserver>>,
}

struct WakeObserver(futures_util::task::AtomicWaker);

impl futures_util::task::ArcWake for WakeObserver {
    fn wake_by_ref(observer: &Arc<Self>) {
        meshway_logging::profile::event("manager.event_queue", "wake");
        observer.0.wake();
    }
}

pub(super) fn channel(count: usize, bytes: usize) -> (Sender, Receiver) {
    let (tx, rx) = mpsc::channel(count);
    let frames = Arc::new(Semaphore::new(count));
    (
        Sender {
            inner: tx,
            bytes: Arc::new(Semaphore::new(bytes)),
            frames: frames.clone(),
            limit: bytes,
        },
        Receiver {
            inner: rx,
            pending: None,
            frames,
            count,
            observer: meshway_logging::profile::enabled()
                .then(|| Arc::new(WakeObserver(Default::default()))),
        },
    )
}

fn capacity(event: &Event) -> usize {
    match event {
        Event::Frame { frame, .. } => frame.payload.capacity(),
        _ => 0,
    }
}

/// Frame and storage permits awaited for one exact group of events.
pub(super) struct Reservation {
    frames: OwnedSemaphorePermit,
    bytes: OwnedSemaphorePermit,
    count: usize,
    size: usize,
}

impl Sender {
    pub fn capacity(&self) -> usize {
        self.frames.available_permits()
    }

    /// Wait until the queue has room for exactly `events` (at most 32), and
    /// hold that room. Room is made only by the consumer dequeuing or
    /// dropping earlier groups; `None` once the queue closed.
    pub fn reserve<'a>(
        &self,
        events: impl IntoIterator<Item = &'a Event>,
    ) -> impl std::future::Future<Output = Option<Reservation>> + Send + 'static {
        let (count, size) = events.into_iter().fold((0, 0), |(count, size), event| {
            (count + 1, size + capacity(event))
        });
        let sender = self.clone();
        async move {
            if count == 0 || count > 32 || size > sender.limit {
                return None;
            }
            let frames = tokio::select! { biased;
                _ = sender.inner.closed() => return None,
                permit = sender.frames.clone().acquire_many_owned(count as u32) => permit.ok()?,
            };
            let bytes = tokio::select! { biased;
                _ = sender.inner.closed() => return None,
                permit = sender.bytes.clone().acquire_many_owned(size as u32) => permit.ok()?,
            };
            Some(Reservation {
                frames,
                bytes,
                count,
                size,
            })
        }
    }

    /// Publish the exact group `reservation` was awaited for.
    pub fn send_reserved(
        &self,
        reservation: Reservation,
        events: Vec<Event>,
    ) -> Result<(), mpsc::error::TrySendError<Vec<Event>>> {
        let size: usize = events.iter().map(capacity).sum();
        if events.len() != reservation.count || size != reservation.size {
            return Err(mpsc::error::TrySendError::Full(events));
        }
        self.inner
            .try_send(Queued {
                events: events.into(),
                _frames: reservation.frames,
                _bytes: reservation.bytes,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(queued) => {
                    mpsc::error::TrySendError::Full(queued.events.into())
                }
                mpsc::error::TrySendError::Closed(queued) => {
                    mpsc::error::TrySendError::Closed(queued.events.into())
                }
            })
    }

    /// Give an already notified owner an extra turn once two receive quanta
    /// remain queued. This is a scheduling hint, never an admission/credit
    /// reservation: a paused consumer still hits the original hard bounds.
    pub fn consumer_turn(&self, incoming: usize) -> bool {
        let available = self.capacity();
        incoming != 0 && (available < incoming || self.inner.max_capacity() - available >= 64)
    }

    pub fn try_send(&self, event: Event) -> Result<(), mpsc::error::TrySendError<Event>> {
        self.try_send_batch(vec![event])
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(mut events) => {
                    mpsc::error::TrySendError::Full(events.pop().expect("single event"))
                }
                mpsc::error::TrySendError::Closed(mut events) => {
                    mpsc::error::TrySendError::Closed(events.pop().expect("single event"))
                }
            })
    }

    pub fn try_send_batch(
        &self,
        events: Vec<Event>,
    ) -> Result<(), mpsc::error::TrySendError<Vec<Event>>> {
        assert!(!events.is_empty() && events.len() <= 32);
        let size: usize = events.iter().map(capacity).sum();
        if self.inner.is_closed() {
            return Err(mpsc::error::TrySendError::Closed(events));
        }
        let Ok(frames) = self
            .frames
            .clone()
            .try_acquire_many_owned(events.len() as u32)
        else {
            return Err(mpsc::error::TrySendError::Full(events));
        };
        let permit = if size <= self.limit {
            self.bytes.clone().try_acquire_many_owned(size as u32).ok()
        } else {
            None
        };
        let Some(permit) = permit else {
            return Err(mpsc::error::TrySendError::Full(events));
        };
        self.inner
            .try_send(Queued {
                events: events.into(),
                _frames: frames,
                _bytes: permit,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(queued) => {
                    mpsc::error::TrySendError::Full(queued.events.into())
                }
                mpsc::error::TrySendError::Closed(queued) => {
                    mpsc::error::TrySendError::Closed(queued.events.into())
                }
            })
    }

    pub async fn send(&self, event: Event) -> Result<(), mpsc::error::SendError<Event>> {
        let size = capacity(&event);
        if size > self.limit {
            return Err(mpsc::error::SendError(event));
        }
        let frames = tokio::select! { biased;
            _ = self.inner.closed() => return Err(mpsc::error::SendError(event)),
            permit = self.frames.clone().acquire_owned() => permit,
        };
        let Ok(frames) = frames else {
            return Err(mpsc::error::SendError(event));
        };
        let permit = tokio::select! { biased;
            _ = self.inner.closed() => return Err(mpsc::error::SendError(event)),
            permit = self.bytes.clone().acquire_many_owned(size as u32) => permit,
        };
        let Ok(permit) = permit else {
            return Err(mpsc::error::SendError(event));
        };
        self.inner
            .send(Queued {
                events: VecDeque::from([event]),
                _frames: frames,
                _bytes: permit,
            })
            .await
            .map_err(|mut error| {
                mpsc::error::SendError(error.0.events.pop_front().expect("single event"))
            })
    }
}

impl Receiver {
    pub async fn recv_many(&mut self, events: &mut Vec<Event>, limit: usize) -> usize {
        if limit == 0 {
            return 0;
        }
        if self.pending.is_none() {
            self.pending = std::future::poll_fn(|cx| {
                meshway_logging::profile::event("manager.event_queue", "poll");
                match &self.observer {
                    Some(observer) => {
                        observer.0.register(cx.waker());
                        let waker = futures_util::task::waker_ref(observer);
                        self.inner
                            .poll_recv(&mut std::task::Context::from_waker(&waker))
                    }
                    None => self.inner.poll_recv(cx),
                }
            })
            .await;
        }
        let mut count = 0;
        while count < limit {
            if self.pending.is_none() {
                self.pending = self.inner.try_recv().ok();
            }
            let Some(pending) = &mut self.pending else {
                break;
            };
            let prefix = pending.events.len().min(limit - count);
            events.extend(pending.events.drain(..prefix));
            count += prefix;
            if pending.events.is_empty() {
                self.pending.take();
            }
        }
        count
    }
    pub fn try_recv(&mut self) -> Result<Event, mpsc::error::TryRecvError> {
        if self.pending.is_none() {
            self.pending = Some(self.inner.try_recv()?);
        }
        let pending = self.pending.as_mut().expect("queued prefix");
        let event = pending.events.pop_front().expect("nonempty batch");
        if pending.events.is_empty() {
            self.pending.take();
        }
        Ok(event)
    }
    pub fn len(&self) -> usize {
        self.count - self.frames.available_permits()
    }
    pub fn close(&mut self) {
        self.inner.close();
    }
}
