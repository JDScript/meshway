//! Exercise tokio-rustls' last-gasp write on a corrupt TLS record, rather than
//! reproducing the WebSocket flush/lifetime test machinery.
use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use meshway_transport::{Frame, Limits, SendContext, SendOutcome, Session};
use meshway_transport_trojan::{Acceptor, Connector};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

#[derive(Default)]
struct Fault {
    block_output: bool,
    blocked: usize,
    output_calls: usize,
    record: Vec<u8>,
    invalidate_on_read: Option<Box<dyn FnOnce() + Send>>,
}

struct FaultIo {
    inner: DuplexStream,
    fault: Arc<Mutex<Fault>>,
}

impl AsyncRead for FaultIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let injected = {
            let mut fault = this.fault.lock().unwrap();
            let length = buffer.remaining().min(fault.record.len());
            if length == 0 {
                None
            } else {
                buffer.put_slice(&fault.record[..length]);
                drop(fault.record.drain(..length));
                Some(fault.invalidate_on_read.take())
            }
        };
        if let Some(invalidate) = injected {
            // Session's read-entry gate was already checked. Invalidate here
            // before rustls processes these bytes and attempts its alert write.
            if let Some(invalidate) = invalidate {
                invalidate();
            }
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buffer)
    }
}

impl AsyncWrite for FaultIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        {
            let mut fault = this.fault.lock().unwrap();
            fault.output_calls += 1;
            if fault.block_output {
                fault.blocked += 1;
                return Poll::Pending;
            }
        }
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.fault.lock().unwrap().output_calls += 1;
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.fault.lock().unwrap().output_calls += 1;
        Pin::new(&mut this.inner).poll_shutdown(cx)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn tls_read_error_output_rechecks_scope_on_both_trojan_provider_roles() {
    for client_side in [false, true] {
        for cancel_on_read in [false, true] {
            let (parameters, prepared) = crate::fixture();
            let connector = Connector::new(&parameters).unwrap();
            let acceptor = Acceptor::new(&prepared).unwrap();
            let client_fault = Arc::new(Mutex::new(Fault::default()));
            let server_fault = Arc::new(Mutex::new(Fault::default()));
            let (client, server) = tokio::io::duplex(64 * 1024);
            let destination = crate::target("service.example");
            let (client, server) = tokio::join!(
                connector.connect(
                    FaultIo {
                        inner: client,
                        fault: client_fault.clone()
                    },
                    &destination
                ),
                acceptor.accept(FaultIo {
                    inner: server,
                    fault: server_fault.clone()
                }),
            );
            let client = client.unwrap();
            let server = server.unwrap().stream;
            let (stream, _peer, fault) = if client_side {
                (client, server, client_fault)
            } else {
                (server, client, server_fault)
            };
            let mut session = Session::stream(
                stream,
                "127.0.0.1:443".parse().unwrap(),
                Limits::new(8192, 1, 8192, 1, 1).unwrap(),
            );
            let context = SendContext::default();
            fault.lock().unwrap().block_output = true;
            let mut frames = VecDeque::from([Frame::new(vec![0xa5; 8192], None)]);
            let admitted = session.sender().try_send(&mut frames, context.clone());
            assert_eq!(admitted.accepted, 1);
            let before = {
                let mut fault = fault.lock().unwrap();
                assert!(fault.blocked > 0);
                fault.block_output = false;
                // A TLS application-data record too short to contain an AEAD
                // tag. Parsing/decryption and alert generation are real rustls.
                fault.record = vec![0x17, 0x03, 0x03, 0x00, 0x01, 0xff];
                if cancel_on_read {
                    let cancel = context.cancellation.clone();
                    fault.invalidate_on_read = Some(Box::new(move || cancel.cancel()));
                }
                fault.output_calls
            };
            // Deliberately no await/yield after admission. The worker cannot
            // close the stream before this receive-triggered TLS output.
            let mut receive = Box::pin(session.receiver().recv());
            assert!(matches!(
                receive
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Ready(Err(_))
            ));
            drop(receive);
            let after = fault.lock().unwrap().output_calls;
            if cancel_on_read {
                assert!(context.cancellation.is_cancelled());
                assert_eq!(
                    after, before,
                    "TLS last-gasp output bypassed the frozen scope"
                );
            } else {
                assert!(
                    after > before,
                    "control case must exercise an actual read-driven TLS write"
                );
            }
            session.shutdown().await.unwrap();
            let completed = admitted.completion.unwrap().wait().await.unwrap();
            assert!(matches!(
                completed.frames[0].outcome,
                SendOutcome::Indeterminate(_)
            ));
        }
    }
}
