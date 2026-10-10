use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use meshway_model::VlessStream;
use meshway_transport::{
    Frame, Limits, SendContext, SendLifetime, SendOutcome, Session, raw::Stream,
};
use meshway_transport_stream::vless::{Acceptor, Connector};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, DuplexStream, ReadBuf};

#[derive(Default)]
struct Gate {
    open: AtomicBool,
    blocked: AtomicUsize,
    output_calls: AtomicUsize,
}

struct Socket {
    inner: DuplexStream,
    gate: Arc<Gate>,
}

impl AsyncRead for Socket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, bytes)
    }
}

impl AsyncWrite for Socket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.gate.output_calls.fetch_add(1, Ordering::SeqCst);
        if !this.gate.open.load(Ordering::SeqCst) {
            this.gate.blocked.fetch_add(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.gate.output_calls.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.gate.output_calls.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

// Deliberately opaque and above the real provider. This can invalidate after
// Session's read-entry check, proving checks reach below WS and TLS as well.
struct InvalidateOnRead {
    inner: Box<dyn Stream>,
    armed: Arc<AtomicBool>,
    invalidate: tokio::sync::watch::Sender<bool>,
}

impl AsyncRead for InvalidateOnRead {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.armed.swap(false, Ordering::SeqCst) {
            this.invalidate.send_replace(true);
        }
        Pin::new(&mut this.inner).poll_read(cx, bytes)
    }
}

impl AsyncWrite for InvalidateOnRead {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[derive(Debug)]
struct Lifetime(tokio::sync::watch::Receiver<bool>);

impl SendLifetime for Lifetime {
    fn is_current(&self) -> bool {
        !*self.0.borrow()
    }
    fn ended(self: Arc<Self>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut state = self.0.clone();
            loop {
                if *state.borrow_and_update() {
                    break;
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
        })
    }
}

#[tokio::test(flavor = "current_thread")]
async fn receiver_first_cannot_flush_expired_ws_tls_output_even_inside_opaque_poll() {
    // Cancellation before read, lifetime before read, lifetime during read.
    for invalidation in 0..3 {
        let (parameters, prepared, _) = crate::fixture(VlessStream::WebSocket);
        let connector = Connector::new(&parameters).unwrap();
        let acceptor = Acceptor::new(&prepared).unwrap();
        let gate = Arc::new(Gate::default());
        gate.open.store(true, Ordering::SeqCst);
        let (local, remote) = tokio::io::duplex(64 * 1024);
        let destination = crate::target("service.example");
        let (client, server) = tokio::join!(
            connector.connect(
                Socket {
                    inner: local,
                    gate: gate.clone()
                },
                &destination
            ),
            acceptor.accept(remote),
        );
        let mut client = client.unwrap();
        let mut server = server.unwrap().stream;
        // Consume the already-written lazy response header before measuring
        // application output. No application response is available.
        {
            let mut bytes = [0; 1];
            let mut read = std::pin::pin!(client.read(&mut bytes));
            assert!(futures_util::poll!(read.as_mut()).is_pending());
        }
        let (invalid, lifetime) = tokio::sync::watch::channel(false);
        let armed = Arc::new(AtomicBool::new(false));
        let mut session = Session::stream(
            InvalidateOnRead {
                inner: client,
                armed: armed.clone(),
                invalidate: invalid.clone(),
            },
            "127.0.0.1:443".parse().unwrap(),
            Limits::new(8192, 2, 16384, 2, 2).unwrap(),
        );
        let context = SendContext {
            lifetime: Some(Arc::new(Lifetime(lifetime))),
            ..SendContext::default()
        };
        gate.open.store(false, Ordering::SeqCst);
        let mut frames = VecDeque::from([Frame::new(vec![0xa5; 8192], None)]);
        let admitted = session.sender().try_send(&mut frames, context.clone());
        assert_eq!(admitted.accepted, 1);
        assert!(gate.blocked.load(Ordering::SeqCst) > 0);
        let calls = gate.output_calls.load(Ordering::SeqCst);
        match invalidation {
            0 => context.cancellation.cancel(),
            1 => {
                invalid.send_replace(true);
            }
            _ => armed.store(true, Ordering::SeqCst),
        }
        gate.open.store(true, Ordering::SeqCst);
        // No await/yield since admission: receiver runs before the worker.
        {
            let mut receive = std::pin::pin!(session.receiver().recv());
            assert!(matches!(
                futures_util::poll!(receive.as_mut()),
                Poll::Ready(Err(_))
            ));
        }
        assert!(context.invalid_reason().is_some());
        assert_eq!(gate.output_calls.load(Ordering::SeqCst), calls);
        session.shutdown().await.unwrap();
        let completed = admitted.completion.unwrap().wait().await.unwrap();
        assert!(matches!(
            completed.frames[0].outcome,
            SendOutcome::Indeterminate(_)
        ));
        let mut received = Vec::new();
        let _ = server.read_to_end(&mut received).await;
        assert!(received.is_empty());
    }
}
