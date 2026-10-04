//! One cached I/O waker fans out to independent read and write task slots.
//! TLS, WebSocket read-driven flushes, and the handshake clock may each keep
//! only one waker internally. They must never replace a caller's other half.
use std::{
    sync::Arc,
    task::{Context, Waker},
};

use futures_util::task::{ArcWake, AtomicWaker, waker};

pub(crate) struct WakeBridge {
    slots: Arc<Slots>,
    waker: Waker,
}

#[derive(Default)]
struct Slots {
    reader: AtomicWaker,
    writer: AtomicWaker,
}

impl ArcWake for Slots {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.reader.wake();
        arc_self.writer.wake();
    }
}

impl WakeBridge {
    pub(crate) fn new() -> Self {
        let slots = Arc::new(Slots::default());
        let waker = waker(slots.clone());
        Self { slots, waker }
    }

    pub(crate) fn read(&self, cx: &Context<'_>) -> Waker {
        self.slots.reader.register(cx.waker());
        self.waker.clone()
    }

    pub(crate) fn write(&self, cx: &Context<'_>) -> Waker {
        self.slots.writer.register(cx.waker());
        self.waker.clone()
    }

    pub(crate) fn wake_all(&self) {
        Slots::wake_by_ref(&self.slots);
    }
}
