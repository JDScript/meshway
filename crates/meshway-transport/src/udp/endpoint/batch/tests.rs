use super::*;
use nix::errno::Errno;
use std::{
    future::poll_fn,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Wake, Waker},
    time::Duration,
};

#[test]
fn aggregate_payload_limits_follow_effective_destination_family() {
    for (peer, limit) in [
        ("127.0.0.1:1", 65507_usize),
        ("[::1]:1", 65527),
        ("[::ffff:127.0.0.1]:1", 65507),
    ] {
        let peer = peer.parse().unwrap();
        for total in [limit - 1, limit, limit + 1, 65535] {
            let first = vec![0; total.div_ceil(2)];
            let last = vec![0; total / 2];
            let packets = [
                DatagramSend {
                    payload: &first,
                    peer,
                },
                DatagramSend {
                    payload: &last,
                    peer,
                },
            ];
            let expected = if total <= limit {
                (2, total)
            } else {
                (1, first.len())
            };
            assert_eq!(gso_prefix(&packets), expected, "{peer}, {total}");
        }
    }
}

#[test]
fn aggregate_stops_at_short_tail_peer_change_empty_and_poll_budget() {
    let peer = "127.0.0.1:1".parse().unwrap();
    let packet = DatagramSend {
        payload: &[0; 100],
        peer,
    };
    let short = DatagramSend {
        payload: &[1; 50],
        peer,
    };
    assert_eq!(gso_prefix(&[packet, short, packet]), (2, 150));
    assert_eq!(
        gso_prefix(&[packet, DatagramSend { payload: &[], peer }, packet]),
        (1, 100)
    );
    assert_eq!(
        gso_prefix(&[
            packet,
            DatagramSend {
                peer: "127.0.0.1:2".parse().unwrap(),
                ..packet
            }
        ]),
        (1, 100)
    );
    assert_eq!(gso_prefix(&[packet; COUNT + 1]), (COUNT, COUNT * 100));
    assert_eq!(gso_prefix(&[]), (0, 0));
}

#[tokio::test]
async fn kernel_sends_at_payload_bounds_and_uses_mmsg_above_them() {
    for (local, mapped, limit) in [
        ("127.0.0.1:0", false, 65507_usize),
        ("[::1]:0", false, 65527),
        ("127.0.0.1:0", true, 65507),
    ] {
        let receiver = UdpSocket::bind(local).await.unwrap();
        let sender = UdpSocket::bind(if mapped { "[::]:0" } else { local })
            .await
            .unwrap();
        let state = State::new(&sender).unwrap();
        let peer = match receiver.local_addr().unwrap() {
            SocketAddr::V4(peer) if mapped => {
                SocketAddr::new(peer.ip().to_ipv6_mapped().into(), peer.port())
            }
            peer => peer,
        };
        for total in [limit - 1, limit, limit + 1] {
            let first = vec![1; total.div_ceil(2)];
            let last = vec![2; total / 2];
            let packets = [
                DatagramSend {
                    payload: &first,
                    peer,
                },
                DatagramSend {
                    payload: &last,
                    peer,
                },
            ];
            let before = state.capabilities();
            assert_eq!(
                tokio::time::timeout(
                    Duration::from_secs(2),
                    poll_fn(|cx| state.poll_send(&sender, cx, &packets))
                )
                .await
                .unwrap()
                .unwrap(),
                2
            );
            let after = state.capabilities();
            assert_eq!(after.udp_segment, before.udp_segment);
            assert_eq!(
                after.gso_sends - before.gso_sends,
                u64::from(before.udp_segment && total <= limit)
            );
            let mut buffer = vec![0; SIZE];
            for packet in packets {
                let (len, _) =
                    tokio::time::timeout(Duration::from_secs(2), receiver.recv_from(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(&buffer[..len], packet.payload);
            }
        }
    }
}

#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn gso_eio_yields_before_ordinary_send_and_preserves_datagrams() {
    for mmsg in [None, Some(false)] {
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let state = State::new(&sender).unwrap();
        {
            let mut caps = state.caps.lock().unwrap();
            caps.udp_segment = true;
            caps.sendmmsg = mmsg;
        }
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut cx = Context::from_waker(&waker);
        // Inject the driver failure at the production syscall-result boundary.
        assert!(
            state
                .send_result(&mut cx, true, false, Err(Errno::EIO.into()))
                .is_pending()
        );
        assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
        assert!(!state.capabilities().udp_segment);
        assert_eq!(state.capabilities().sendmmsg, mmsg);
        let mut buffer = [0; 8];
        assert_eq!(
            receiver.try_recv_from(&mut buffer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        let peer = receiver.local_addr().unwrap();
        let packets = [
            DatagramSend {
                payload: b"first",
                peer,
            },
            DatagramSend {
                payload: b"last",
                peer,
            },
        ];
        let mut sent = 0;
        while sent < packets.len() {
            sent += tokio::time::timeout(
                Duration::from_secs(2),
                poll_fn(|cx| state.poll_send(&sender, cx, &packets[sent..])),
            )
            .await
            .unwrap()
            .unwrap();
        }
        for packet in packets {
            let (len, source) =
                tokio::time::timeout(Duration::from_secs(2), receiver.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&buffer[..len], packet.payload);
            assert_eq!(source, sender.local_addr().unwrap());
        }
        assert_eq!(state.capabilities().gso_sends, 0);
        assert_eq!(
            state.capabilities().mmsg_sends,
            if mmsg.is_none() { 1 } else { 0 }
        );
    }
}

#[tokio::test]
async fn ordinary_eio_and_all_emsgsize_are_real_errors() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let state = State::new(&socket).unwrap();
    state.caps.lock().unwrap().udp_segment = true;
    let mut cx = Context::from_waker(Waker::noop());
    for (gso, mmsg, error) in [
        (false, false, Errno::EIO),
        (false, true, Errno::EIO),
        (false, false, Errno::EMSGSIZE),
        (false, true, Errno::EMSGSIZE),
        (true, false, Errno::EMSGSIZE),
    ] {
        let Poll::Ready(Err(actual)) = state.send_result(&mut cx, gso, mmsg, Err(error.into()))
        else {
            panic!("{error} must not trigger fallback");
        };
        assert_eq!(actual.raw_os_error(), Some(error as i32));
        assert!(state.capabilities().udp_segment);
        assert_eq!(state.capabilities().sendmmsg, None);
    }
    // The receive/probe classifier must not accept either error.
    assert!(!unsupported(&Errno::EIO.into()));
    assert!(!unsupported(&Errno::EMSGSIZE.into()));
}

#[tokio::test]
async fn scratch_ranges_survive_partial_drain_and_preserve_sources() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let state = State::new(&socket).unwrap();
    let first_source = "127.0.0.1:1".parse().unwrap();
    let second_source = "[::1]:2".parse().unwrap();
    {
        let mut rx = state.rx.lock().unwrap();
        rx.scratch[0][..5].copy_from_slice(b"aabbc");
        rx.scratch[1][..2].copy_from_slice(b"dd");
        for (slot, range, source) in [
            (0, 0..2, first_source),
            (0, 2..4, first_source),
            (0, 4..5, first_source),
            (1, 0..2, second_source),
        ] {
            rx.pending.push_back(Received {
                slot,
                read: DatagramRead {
                    source,
                    copied_len: range.len(),
                    original_len: Some(range.len()),
                    truncated: false,
                },
                range,
            });
        }
    }
    // New socket input must not overwrite scratch until both slots are drained.
    sender
        .send_to(b"new", socket.local_addr().unwrap())
        .await
        .unwrap();
    let mut buffers = vec![vec![0; 2]];
    let reads = poll_fn(|cx| state.poll_recv(&socket, cx, &mut buffers))
        .await
        .unwrap();
    assert_eq!(buffers[0], b"aa");
    assert_eq!(reads[0].source, first_source);
    let mut small = [0; 1];
    let read = poll_fn(|cx| state.poll_recv_one(&socket, cx, &mut small))
        .await
        .unwrap();
    assert_eq!(small, *b"b");
    assert!(read.truncated);
    assert_eq!(read.original_len, Some(2));
    for (expected, source) in [
        (b"c".as_slice(), first_source),
        (b"dd".as_slice(), second_source),
        (b"new".as_slice(), sender.local_addr().unwrap()),
    ] {
        let mut buffer = [0; 8];
        let read = tokio::time::timeout(
            Duration::from_secs(2),
            poll_fn(|cx| state.poll_recv_one(&socket, cx, &mut buffer)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buffer[..read.copied_len], expected);
        assert_eq!(read.source, source);
        assert!(!read.truncated);
    }
}
