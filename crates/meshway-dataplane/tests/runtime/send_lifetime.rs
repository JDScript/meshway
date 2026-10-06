//! Accepted transport work retains the real Control lifetime independently of
//! the caller polling (or dropping) the formal carrier send future.
use super::support::secure::SecureMesh;
use meshway_dataplane::carrier;
use meshway_transport::{Limits, ReceiveError, Session};
use std::{
    future::{Future, poll_fn},
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, DuplexStream, ReadBuf},
    sync::Notify,
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct GateState {
    open: bool,
    write_polls: usize,
    written: usize,
    writer: Option<Waker>,
}

#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    polled: Notify,
}

impl Gate {
    async fn worker_polled(&self) {
        loop {
            let notified = self.polled.notified();
            if self.state.lock().unwrap().write_polls >= 2 {
                return;
            }
            notified.await;
        }
    }

    fn open(&self) {
        let writer = {
            let mut state = self.state.lock().unwrap();
            state.open = true;
            state.writer.take()
        };
        if let Some(writer) = writer {
            writer.wake();
        }
    }
}

/// Only raw readiness is injected: framing, admission, ownership, lifetime and
/// cancellation all use the production Session and carrier adapter.
struct GatedStream {
    stream: DuplexStream,
    gate: Arc<Gate>,
}

impl AsyncRead for GatedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for GatedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let open = {
            let mut state = self.gate.state.lock().unwrap();
            state.write_polls += 1;
            state.writer = Some(cx.waker().clone());
            state.open
        };
        self.gate.polled.notify_one();
        if !open {
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(written)) = result {
            self.gate.state.lock().unwrap().written += written;
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Clone, Copy)]
enum Invalidation {
    ControlStop,
    DropSend,
}

async fn accepted_send_is_fenced(invalidation: Invalidation) {
    let mut fixture = SecureMesh::start().await;
    // This HS1 frame uses the normally enrolled keys and the installed complete
    // path. No synthetic snapshot, lifetime or protocol envelope is constructed.
    let pending = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let snapshot = fixture.mesh.a.control.execution_snapshot().unwrap();
    let gate = Arc::new(Gate::default());
    let (stream, mut raw_peer) = tokio::io::duplex(1);
    let (mut sender, mut receiver) = Session::stream(
        GatedStream {
            stream,
            gate: gate.clone(),
        },
        "127.0.0.1:12345".parse().unwrap(),
        Limits::new(128 * 1024, 1, 1024 * 1024, 1, 1).unwrap(),
    )
    .into_split();
    let mut sending = Some(Box::pin(carrier::send(&mut sender, pending, None)));
    poll_fn(|cx| {
        assert!(sending.as_mut().unwrap().as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    // Current-thread runtime: the first poll was inline admission. The next
    // observed raw-write poll must be the independently scheduled send worker.
    assert_eq!(gate.state.lock().unwrap().write_polls, 1);
    timeout(WAIT, gate.worker_polled()).await.unwrap();
    assert_eq!(gate.state.lock().unwrap().written, 0);
    snapshot.ensure_current().unwrap();
    let mut byte = [0];
    {
        let mut read = Box::pin(raw_peer.read(&mut byte));
        poll_fn(|cx| {
            assert!(read.as_mut().poll(cx).is_pending(), "raw peer closed early");
            Poll::Ready(())
        })
        .await;
    }

    match invalidation {
        Invalidation::ControlStop => {
            // Keep `sending` alive and never poll it while Control invalidates
            // the worker-owned lifetime and raw readiness becomes available.
            timeout(WAIT, fixture.mesh.a.control.stop())
                .await
                .unwrap()
                .unwrap();
            assert!(snapshot.ensure_current().is_err());
        }
        Invalidation::DropSend => {
            drop(sending.take());
            // Cancellation here must come from the adapter's DropGuard, not
            // from Control invalidation or dropping either transport half.
            snapshot.ensure_current().unwrap();
        }
    }
    gate.open();
    assert_eq!(
        timeout(WAIT, raw_peer.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0,
        "accepted send wrote after invalidation"
    );
    assert_eq!(gate.state.lock().unwrap().written, 0);
    assert!(matches!(
        timeout(WAIT, receiver.recv()).await.unwrap(),
        Err(ReceiveError::Closed)
    ));
    // Only after observing EOF and zero raw bytes may the caller resume its
    // outer future. Its cancellation cannot be what protected the raw writer.
    if let Some(sending) = sending.take() {
        assert!(matches!(
            timeout(WAIT, sending).await.unwrap(),
            Err(carrier::Error::Cancelled)
        ));
    }
    drop(sending);
    assert!(sender.is_closed());
    timeout(WAIT, sender.shutdown()).await.unwrap().unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn control_stop_fences_accepted_worker_while_outer_send_is_alive_but_unpolled() {
    accepted_send_is_fenced(Invalidation::ControlStop).await;
}

#[tokio::test]
async fn dropping_outer_send_cancels_accepted_worker_before_raw_gate_opens() {
    accepted_send_is_fenced(Invalidation::DropSend).await;
}

#[derive(Debug)]
struct SnapshotLifetime(meshway_control::ExecutionSnapshot);
impl meshway_transport::SendLifetime for SnapshotLifetime {
    fn is_current(&self) -> bool {
        self.0.ensure_current().is_ok()
    }
    fn invalidated(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move { self.0.invalidated().await })
    }
}

#[tokio::test]
async fn shared_send_receipt_starts_after_sender_lock_wait_and_requires_local_complete() {
    let mut fixture = SecureMesh::start().await;
    let pending = fixture
        .a
        .start(fixture.mesh.demand, fixture.mesh.path.clone())
        .unwrap();
    let channel = Arc::new(SnapshotLifetime(
        fixture.mesh.a.control.execution_snapshot().unwrap(),
    ));
    let peer = "127.0.0.1:12345".parse().unwrap();
    let (stream, _raw_peer) = tokio::io::duplex(65536);
    let (sender, _receiver) = Session::stream(
        stream,
        peer,
        Limits::new(128 * 1024, 1, 1024 * 1024, 1, 1).unwrap(),
    )
    .into_split();
    let sender = tokio::sync::Mutex::new(sender);
    let locked = sender.lock().await;
    let mut sending = Box::pin(carrier::send_shared_with_receipt(
        &sender, pending, peer, channel,
    ));
    poll_fn(|cx| {
        assert!(sending.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let unlocked_at = std::time::Instant::now();
    drop(locked);
    let receipt = timeout(WAIT, sending).await.unwrap().unwrap();
    assert!(receipt.admitted_at >= unlocked_at);
    assert!(receipt.admitted_at <= std::time::Instant::now());
    sender.lock().await.shutdown().await.unwrap();
    fixture.close().await;
}
