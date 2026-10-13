use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    sync::Mutex,
    task::{Context, Poll},
};

use crate::spec::SocketOptions;
use crate::{
    AdmissionStop, CancelReason, Frame, Limits, ReceiveError, SendContext, SendFailure,
    SendOutcome, Session,
    raw::{Datagram, DatagramRead},
};
use socket2::SockRef;

#[cfg(target_os = "linux")]
#[tokio::test]
async fn batch_buffers_use_kernel_clamped_defaults_and_preserve_explicit_sizes() {
    for (receive_buffer, send_buffer) in [(0, 0), (16 * 1024, 32 * 1024)] {
        let expected = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let expected = SockRef::from(&expected);
        expected
            .set_recv_buffer_size(if receive_buffer == 0 {
                7 * 1024 * 1024
            } else {
                receive_buffer as usize
            })
            .unwrap();
        expected
            .set_send_buffer_size(if send_buffer == 0 {
                7 * 1024 * 1024
            } else {
                send_buffer as usize
            })
            .unwrap();
        let endpoint = UdpTransport::adopt_batch(
            UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            &SocketOptions {
                receive_buffer,
                send_buffer,
            },
        )
        .unwrap();
        let actual = SockRef::from(&endpoint.socket);
        assert_eq!(
            actual.recv_buffer_size().unwrap(),
            expected.recv_buffer_size().unwrap()
        );
        assert_eq!(
            actual.send_buffer_size().unwrap(),
            expected.send_buffer_size().unwrap()
        );
    }
}
use tokio::net::UdpSocket;

use super::UdpTransport;

async fn endpoint() -> UdpTransport {
    UdpTransport::adopt(
        UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        &SocketOptions::default(),
    )
    .unwrap()
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn kernel_gso_gro_mmsg_and_single_fallback_preserve_datagrams() {
    use crate::raw::DatagramSend;
    use std::future::poll_fn;
    use std::time::Duration;
    for local in ["127.0.0.1:0", "[::1]:0"] {
        let left = UdpTransport::adopt_batch(
            UdpSocket::bind(local).await.unwrap(),
            &SocketOptions::default(),
        )
        .unwrap();
        let right = UdpTransport::adopt_batch(
            UdpSocket::bind(local).await.unwrap(),
            &SocketOptions::default(),
        )
        .unwrap();
        for mode in 0..3 {
            if mode >= 1 {
                left.batch
                    .as_ref()
                    .unwrap()
                    .caps
                    .lock()
                    .unwrap()
                    .udp_segment = false;
                nix::sys::socket::setsockopt(
                    &right.socket,
                    nix::sys::socket::sockopt::UdpGroSegment,
                    &false,
                )
                .unwrap();
                right.batch.as_ref().unwrap().caps.lock().unwrap().udp_gro = false;
            }
            if mode == 2 {
                left.batch.as_ref().unwrap().caps.lock().unwrap().sendmmsg = Some(false);
                right.batch.as_ref().unwrap().caps.lock().unwrap().recvmmsg = Some(false);
            }
            let payloads = [vec![1; 1200], vec![2; 1200], vec![3; 900]];
            let sends: Vec<_> = payloads
                .iter()
                .map(|payload| DatagramSend {
                    payload,
                    peer: right.local_addr().unwrap(),
                })
                .collect();
            let mut sent = 0;
            while sent < sends.len() {
                sent += poll_fn(|cx| left.poll_send_batch(cx, &sends[sent..]))
                    .await
                    .unwrap();
            }
            let mut received = 0;
            let mut buffers = vec![vec![0; 1300]; 2];
            while received < 2 {
                let reads = tokio::time::timeout(
                    Duration::from_secs(2),
                    poll_fn(|cx| right.poll_recv_batch(cx, &mut buffers[..(2 - received)])),
                )
                .await
                .unwrap()
                .unwrap();
                for (read, buffer) in reads.iter().zip(&buffers) {
                    assert_eq!(read.source, left.local_addr().unwrap());
                    assert!(!read.truncated);
                    assert_eq!(&buffer[..read.copied_len], payloads[received]);
                    received += 1;
                }
            }
            let mut small = [0; 100];
            let read = right.recv_from(&mut small).await.unwrap();
            assert!(read.truncated);
            assert_eq!(read.original_len, Some(900));
            assert_eq!(small, [3; 100]);
            let left_caps = left.capabilities();
            let right_caps = right.capabilities();
            println!("{local} mode {mode}: send {left_caps:?}; receive {right_caps:?}");
            if mode == 0 && left_caps.udp_segment && right_caps.udp_gro {
                assert!(left_caps.gso_sends > 0);
                assert!(right_caps.gro_receives > 0);
            }
            if mode == 1 {
                assert_eq!(left_caps.sendmmsg, Some(true));
                assert_eq!(right_caps.recvmmsg, Some(true));
            }
        }
    }
}

#[tokio::test]
async fn raw_endpoint_reports_real_truncation_exact_boundary_and_source() {
    let sender = endpoint().await;
    let receiver = endpoint().await;
    let destination = receiver.local_addr().unwrap();
    let source = sender.local_addr().unwrap();
    // Object-safe use shares the exact production receive implementation.
    let receiver: Box<dyn Datagram> = Box::new(receiver);
    let mut buffer = [0; 8];
    for size in [9, 8, 0, 1] {
        sender.send_to(&vec![7; size], destination).await.unwrap();
        let read = crate::raw::recv_from(receiver.as_ref(), &mut buffer)
            .await
            .unwrap();
        assert_eq!(read.source, source);
        assert_eq!(read.copied_len, size.min(buffer.len()));
        assert_eq!(read.truncated, size > buffer.len());
        #[cfg(target_os = "linux")]
        assert_eq!(read.original_len, Some(size));
        #[cfg(target_os = "macos")]
        assert_eq!(read.original_len, (size <= buffer.len()).then_some(size));
        assert!(buffer[..read.copied_len].iter().all(|byte| *byte == 7));
    }
}

#[tokio::test]
async fn framed_endpoint_rejects_empty_and_truncated_without_poisoning_other_peers() {
    let sender = endpoint().await;
    let other = endpoint().await;
    let receiver = endpoint().await;
    let destination = receiver.local_addr().unwrap();
    let receiver: Box<dyn Datagram> = Box::new(receiver);
    let mut session = Session::datagram(receiver, Limits::new(8, 4, 32, 2, 2).unwrap());
    sender.send_to(&[1; 9], destination).await.unwrap();
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Truncated(_))
    ));
    sender.send_to(&[], destination).await.unwrap();
    assert!(matches!(
        session.receiver().recv().await,
        Err(ReceiveError::Empty)
    ));
    other.send_to(&[2; 8], destination).await.unwrap();
    let frame = session.receiver().recv().await.unwrap();
    assert_eq!(frame.payload, [2; 8]);
    assert_eq!(frame.peer, Some(other.local_addr().unwrap()));
    assert!(!session.sender().is_closed());
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn framed_udp_batches_preserve_boundaries_and_actual_reply_peers() {
    let left = endpoint().await;
    let right = endpoint().await;
    let left_address = left.local_addr().unwrap();
    let right_address = right.local_addr().unwrap();
    let limits = Limits::new(64, 4, 256, 2, 2).unwrap();
    let mut left = left.into_session(limits);
    let mut right = right.into_session(limits);
    let mut outbound = VecDeque::from([
        Frame::new(b"first".to_vec(), Some(right_address)),
        Frame::new(b"second".to_vec(), Some(right_address)),
    ]);
    let accepted = left
        .sender()
        .try_send(&mut outbound, SendContext::default());
    assert_eq!(accepted.accepted, 2);
    let first = right.receiver().recv().await.unwrap();
    let second = right.receiver().recv().await.unwrap();
    assert_eq!(first.peer, Some(left_address));
    assert_eq!(second.peer, Some(left_address));
    let mut replies = VecDeque::from([first, second]);
    let replies = right
        .sender()
        .try_send(&mut replies, SendContext::default());
    assert_eq!(left.receiver().recv().await.unwrap().payload, b"first");
    assert_eq!(left.receiver().recv().await.unwrap().payload, b"second");
    for completion in [accepted.completion.unwrap(), replies.completion.unwrap()] {
        assert!(
            completion
                .wait()
                .await
                .unwrap()
                .frames
                .iter()
                .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
        );
    }
    left.shutdown().await.unwrap();
    right.shutdown().await.unwrap();
}

#[tokio::test]
async fn datagram_admission_requires_peer_and_enforces_frame_limit() {
    let transport = endpoint().await;
    let mut session = transport.into_session(Limits::new(8, 2, 16, 2, 2).unwrap());
    let mut frames = VecDeque::from([Frame::new(vec![1], None)]);
    let admission = session
        .sender()
        .try_send(&mut frames, SendContext::default());
    assert_eq!(admission.stop, Some(AdmissionStop::MissingPeer));
    assert_eq!(frames.len(), 1);
    frames[0].peer = Some("127.0.0.1:12345".parse().unwrap());
    frames[0].payload = vec![0; 9].into();
    assert_eq!(
        session
            .sender()
            .try_send(&mut frames, SendContext::default())
            .stop,
        Some(AdmissionStop::Oversized {
            size: 9,
            maximum: 8
        })
    );
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn applies_buffers_and_rejects_connected_or_invalid_socket_options() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let options = SocketOptions {
        receive_buffer: 16 * 1024,
        send_buffer: 16 * 1024,
    };
    let transport = UdpTransport::adopt(socket, &options).unwrap();
    let socket = transport.into_inner();
    assert!(SockRef::from(&socket).recv_buffer_size().unwrap() >= options.receive_buffer as usize);
    assert!(SockRef::from(&socket).send_buffer_size().unwrap() >= options.send_buffer as usize);
    socket
        .connect(endpoint().await.local_addr().unwrap())
        .await
        .unwrap();
    assert_eq!(
        UdpTransport::adopt(socket, &options).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let invalid = SocketOptions {
        receive_buffer: 0,
        send_buffer: u32::MAX,
    };
    assert_eq!(
        UdpTransport::adopt(socket, &invalid).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

/// Only gates a raw send poll; successful I/O still delegates to the actual
/// native provider. The owner is the production session implementation.
struct BlockFirstSend {
    endpoint: UdpTransport,
    attempted: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl Datagram for BlockFirstSend {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.endpoint.local_addr()
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if payload == b"blocked" {
            if let Some(attempted) = self.attempted.lock().unwrap().take() {
                let _ = attempted.send(());
            }
            Poll::Pending
        } else {
            self.endpoint.poll_send_to(cx, payload, peer)
        }
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        self.endpoint.poll_recv_from(cx, buffer)
    }
}

#[tokio::test]
async fn datagram_invalidation_does_not_cancel_unrelated_queued_scope() {
    let sender = endpoint().await;
    let receiver = endpoint().await;
    let destination = receiver.local_addr().unwrap();
    let (attempted, started) = tokio::sync::oneshot::channel();
    let sender = BlockFirstSend {
        endpoint: sender,
        attempted: Mutex::new(Some(attempted)),
    };
    let mut session = Session::datagram(sender, Limits::new(32, 4, 128, 2, 2).unwrap());
    let cancelled = SendContext::default();
    let mut first = VecDeque::from([Frame::new(b"blocked".to_vec(), Some(destination))]);
    let first = session.sender().try_send(&mut first, cancelled.clone());
    let mut second = VecDeque::from([Frame::new(b"valid".to_vec(), Some(destination))]);
    let second = session
        .sender()
        .try_send(&mut second, SendContext::default());
    started.await.unwrap();
    cancelled.cancellation.cancel();
    assert!(matches!(
        first.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::Indeterminate(SendFailure::Cancelled(CancelReason::ScopeCancelled))
    ));
    assert!(matches!(
        second.completion.unwrap().wait().await.unwrap().frames[0].outcome,
        SendOutcome::LocalComplete
    ));
    let mut buffer = [0; 32];
    let received = receiver.recv_from(&mut buffer).await.unwrap();
    assert_eq!(&buffer[..received.copied_len], b"valid");
    assert!(!session.sender().is_closed());
    assert_eq!(session.sender().available_budget().operations, 2);
    session.shutdown().await.unwrap();
}

/// Run in a disposable network namespace whose loopback carries the 1357-byte
/// path MTU measured through a real Cloudflare WARP client, e.g.
/// `unshare -rn sh -c 'ip link set lo mtu 1357 up && MESHWAY_TRANSPORT_ISOLATED=1 <test> --ignored --exact ...'`.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires an isolated network namespace whose loopback MTU is 1357"]
async fn df_datagrams_learn_the_path_mtu_and_keep_gso_below_it() {
    use crate::raw::{Datagram, DatagramSend};
    use std::future::poll_fn;
    assert_eq!(
        std::env::var("MESHWAY_TRANSPORT_ISOLATED").as_deref(),
        Ok("1")
    );
    let left = UdpTransport::adopt_batch(
        UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        &SocketOptions::default(),
    )
    .unwrap();
    let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = right.local_addr().unwrap();
    let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    probe.connect(peer).unwrap();
    assert_eq!(super::connected_path_mtu(&probe, true).unwrap(), 1357);
    // 1357 - 28: the largest datagram leaves whole; one byte more never
    // becomes fragments.
    let fits = vec![1; 1329];
    let over = vec![2; 1330];
    assert_eq!(left.send_to(&fits, peer).await.unwrap(), 1329);
    let error = left.send_to(&over, peer).await.unwrap_err();
    assert_eq!(
        error.raw_os_error(),
        Some(nix::errno::Errno::EMSGSIZE as i32)
    );
    let mut buffer = vec![0; 2048];
    let (len, _) = right.recv_from(&mut buffer).await.unwrap();
    assert_eq!(len, 1329);
    let segment = |size| -> Vec<Vec<u8>> { (0..4).map(|_| vec![3; size]).collect() };
    for (size, delivered) in [(1366, false), (1300, true)] {
        let payloads = segment(size);
        let sends: Vec<_> = payloads
            .iter()
            .map(|payload| DatagramSend { payload, peer })
            .collect();
        let result = poll_fn(|cx| left.poll_send_batch(cx, &sends)).await;
        if delivered {
            assert_eq!(result.unwrap(), sends.len());
            for _ in 0..sends.len() {
                let (len, _) = right.recv_from(&mut buffer).await.unwrap();
                assert_eq!(len, size);
            }
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                error.raw_os_error(),
                Some(nix::errno::Errno::EMSGSIZE as i32)
            );
        }
        let caps = left.capabilities();
        assert!(caps.udp_segment, "a size rejection never disables GSO");
    }
    let caps = left.capabilities();
    assert_eq!(caps.gso_size_rejections, 1);
    assert!(caps.gso_sends >= 1, "the fitting batch still used GSO");
    assert!(
        right.try_recv_from(&mut buffer).is_err(),
        "nothing oversized arrived, not even as fragments"
    );
}

/// Same isolated namespace as above (loopback MTU 1357).
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires an isolated network namespace whose loopback MTU is 1357"]
async fn only_a_datagram_no_ethernet_path_carries_whole_may_fragment() {
    use crate::raw::{Datagram, DatagramSend};
    use std::future::poll_fn;
    assert_eq!(
        std::env::var("MESHWAY_TRANSPORT_ISOLATED").as_deref(),
        Ok("1")
    );
    let left = UdpTransport::adopt_batch(
        UdpSocket::bind("127.0.0.1:0").await.unwrap(),
        &SocketOptions::default(),
    )
    .unwrap();
    let right = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let peer = right.local_addr().unwrap();
    let mut buffer = vec![0; 8192];
    // A data-sized frame above the route MTU fails: never fragments.
    let data = vec![1; 1452];
    assert!(left.send_to(&data, peer).await.is_err());
    // A 4 KiB control message arrives whole, reassembled from fragments.
    let message = vec![2; 4096];
    for _ in 0..2 {
        let sends = [DatagramSend {
            payload: &message,
            peer,
        }];
        assert_eq!(
            poll_fn(|cx| left.poll_send_batch(cx, &sends))
                .await
                .unwrap(),
            1
        );
        let (len, _) = right.recv_from(&mut buffer).await.unwrap();
        assert_eq!(len, 4096);
    }
    assert_eq!(left.send_to(&message, peer).await.unwrap(), 4096);
    assert_eq!(right.recv_from(&mut buffer).await.unwrap().0, 4096);
    // DF is back for everything else.
    assert!(left.send_to(&data, peer).await.is_err());
}
