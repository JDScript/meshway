#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

//! Explicit kernel fixture, never part of ordinary test runs.
//!
//! Place this precompiled Linux test executable at
//! `.meshway/tun-fixture/linux-tun-tests` (matching the container libc, or static
//! musl), then run from the repository root:
//!
//! ```text
//! docker run --rm --network none --cap-drop ALL --cap-add NET_ADMIN \
//!   --device /dev/net/tun:/dev/net/tun --read-only \
//!   --sysctl net.ipv6.conf.all.disable_ipv6=0 \
//!   --sysctl net.ipv6.conf.default.disable_ipv6=0 \
//!   --env MESHWAY_TUN_TEST_ISOLATED=1 \
//!   --mount type=bind,src="$PWD/.meshway/tun-fixture",dst=/fixtures,readonly \
//!   debian:trixie-slim /fixtures/linux-tun-tests --ignored --exact \
//!   owned_devices_ipv4_ipv6_packet_io --nocapture
//! ```
//!
//! Requires the Docker Linux host to provide `/dev/net/tun`. There are no shell,
//! iproute2, extra test-library, NET_RAW, or privileged-container requirements.

use std::collections::BTreeSet;
use std::future::{Future, poll_fn};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::task::Poll;
use std::time::Duration;

use meshway_tun::{Error, Options, PacketBatch, Tun};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::time::{sleep, timeout};

mod upstream;

const MTU: usize = 1500;
const WAIT: Duration = Duration::from_secs(5);
const LOCAL_V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 1);
const REMOTE_V4: Ipv4Addr = Ipv4Addr::new(192, 0, 2, 2);
const LOCAL_V6: Ipv6Addr = Ipv6Addr::new(0xfd42, 0x6d65, 0x7368, 0, 0, 0, 0, 1);
const REMOTE_V6: Ipv6Addr = Ipv6Addr::new(0xfd42, 0x6d65, 0x7368, 0, 0, 0, 0, 2);

#[tokio::test]
#[ignore = "requires isolated Linux namespace, NET_ADMIN and MESHWAY_TUN_TEST_ISOLATED=1"]
async fn offload_capabilities_and_normalized_packet_io() {
    verify_offload().await;
}

async fn verify_offload() {
    assert_eq!(
        std::env::var("MESHWAY_TUN_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    let tun = Tun::create(
        Options::new(MTU)
            .unwrap()
            .with_batch_io(true)
            .with_ipv4(LOCAL_V4, 30)
            .unwrap()
            .with_ipv6(LOCAL_V6, 126)
            .unwrap(),
    )
    .unwrap();
    println!("TUN capabilities: {:?}", tun.capabilities());
    wait_for_ipv6(tun.name()).await;
    cancel_empty_receive(&tun).await;
    exchange(&tun, LOCAL_V4.into(), REMOTE_V4.into()).await;
    exchange(&tun, LOCAL_V6.into(), REMOTE_V6.into()).await;
    tcp_offload(&tun, LOCAL_V4.into(), REMOTE_V4.into()).await;
    tcp_offload(&tun, LOCAL_V6.into(), REMOTE_V6.into()).await;
    for (local, remote) in [
        (IpAddr::V4(LOCAL_V4), IpAddr::V4(REMOTE_V4)),
        (IpAddr::V6(LOCAL_V6), IpAddr::V6(REMOTE_V6)),
    ] {
        let socket = UdpSocket::bind(SocketAddr::new(local, 0)).await.unwrap();
        let port = socket.local_addr().unwrap().port();
        socket
            .send_to(b"batch", SocketAddr::new(remote, 42424))
            .await
            .unwrap();
        let mut batch = PacketBatch::default();
        timeout(WAIT, async {
            loop {
                tun.recv_batch(&mut batch).await.unwrap();
                if batch.packets().iter().any(|packet| {
                    fixture_udp_payload(packet, local, remote, port, 42424) == Some(b"batch")
                }) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let packets: Vec<_> = (0..3)
            .map(|_| udp_packet(remote, local, 42424, port, b"batch"))
            .collect();
        let refs: Vec<_> = packets.iter().map(Vec::as_slice).collect();
        let sent = timeout(WAIT, tun.send_batch(&refs)).await.unwrap().unwrap();
        assert_eq!(sent, if tun.capabilities().uso4 { 3 } else { 1 });
        for _ in 0..sent {
            let mut buffer = [0; 100];
            let (len, _) = timeout(WAIT, socket.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&buffer[..len], b"batch");
        }

        // Upstream GRO can join indices 0 and 2 across another flow. The public
        // API must still write only a prefix and report its exact packet count.
        let other = UdpSocket::bind(SocketAddr::new(local, 0)).await.unwrap();
        let other_packet = udp_packet(
            remote,
            local,
            42424,
            other.local_addr().unwrap().port(),
            b"other",
        );
        let mixed = [&packets[0][..], &other_packet[..], &packets[1][..]];
        assert_eq!(tun.send_batch(&mixed).await.unwrap(), 1);
        let mut buffer = [0; 100];
        assert_eq!(socket.recv(&mut buffer).await.unwrap(), 5);
        assert!(
            timeout(Duration::from_millis(30), socket.recv(&mut buffer))
                .await
                .is_err()
        );
        assert!(
            timeout(Duration::from_millis(30), other.recv(&mut buffer))
                .await
                .is_err()
        );
        assert_eq!(tun.send_batch(&mixed[1..]).await.unwrap(), 1);
        assert_eq!(other.recv(&mut buffer).await.unwrap(), 5);
        assert_eq!(tun.send_batch(&mixed[2..]).await.unwrap(), 1);
        assert_eq!(socket.recv(&mut buffer).await.unwrap(), 5);
    }
    upstream::reject_invalid_kernel_headers().await;
}

async fn tcp_offload(tun: &Tun, local: IpAddr, remote: IpAddr) {
    let (delivered, delivery) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut socket = tokio::net::TcpStream::connect(SocketAddr::new(remote, 42424))
            .await
            .unwrap();
        socket.write_all(&vec![0x5a; 8192]).await.unwrap();
        let mut incoming = vec![0; 3000];
        socket.read_exact(&mut incoming).await.unwrap();
        assert!(incoming.iter().all(|byte| *byte == 0x6b));
        delivered.send(()).unwrap();
        // Keep the socket open until the test has consumed the payload.
        std::future::pending::<()>().await;
    });
    let ip = if local.is_ipv4() { 20 } else { 40 };
    let mut buffer = vec![0; MTU];
    let syn = timeout(WAIT, async {
        loop {
            let n = tun.recv(&mut buffer).await.unwrap();
            if n >= ip + 20
                && buffer[0] >> 4 == if local.is_ipv4() { 4 } else { 6 }
                && buffer[if local.is_ipv4() { 9 } else { 6 }] == 6
                && buffer[ip + 13] & 2 != 0
            {
                break buffer[..n].to_vec();
            }
        }
    })
    .await
    .unwrap();
    upstream::assert_checksums(&syn);
    let sequence = u32::from_be_bytes(syn[ip + 4..ip + 8].try_into().unwrap());
    let port = u16::from_be_bytes(syn[ip..ip + 2].try_into().unwrap());
    let reply = tcp_reply(
        remote,
        local,
        port,
        sequence.wrapping_add(1),
        true,
        (0, &[]),
    );
    tun.send(&reply).await.unwrap();
    let mut received = 0;
    let mut maximum_batch = 0;
    let mut batch = PacketBatch::default();
    timeout(WAIT, async {
        while received < 8192 {
            tun.recv_batch(&mut batch).await.unwrap();
            maximum_batch = maximum_batch.max(batch.packets().len());
            for packet in batch.packets() {
                if packet.len() < ip + 20
                    || packet[0] >> 4 != if local.is_ipv4() { 4 } else { 6 }
                    || packet[if local.is_ipv4() { 9 } else { 6 }] != 6
                {
                    continue;
                }
                let header = ip + usize::from(packet[ip + 12] >> 4) * 4;
                assert!(packet.len() <= MTU);
                let mut pseudo = Vec::new();
                if local.is_ipv4() {
                    assert_eq!(checksum(&packet[..ip]), 0);
                    pseudo.extend_from_slice(&packet[12..20]);
                    pseudo.extend_from_slice(&[0, 6]);
                    pseudo.extend_from_slice(&((packet.len() - ip) as u16).to_be_bytes());
                } else {
                    pseudo.extend_from_slice(&packet[8..40]);
                    pseudo.extend_from_slice(&((packet.len() - ip) as u32).to_be_bytes());
                    pseudo.extend_from_slice(&[0, 0, 0, 6]);
                }
                pseudo.extend_from_slice(&packet[ip..]);
                assert_eq!(checksum(&pseudo), 0);
                if packet.len() == header {
                    continue;
                }
                let actual_sequence =
                    u32::from_be_bytes(packet[ip + 4..ip + 8].try_into().unwrap());
                assert_eq!(actual_sequence, sequence.wrapping_add(1 + received as u32));
                assert!(packet[header..].iter().all(|byte| *byte == 0x5a));
                received += packet.len() - header;
            }
            let ack = tcp_reply(
                remote,
                local,
                port,
                sequence.wrapping_add(1 + received as u32),
                false,
                (0, &[]),
            );
            tun.send(&ack).await.unwrap();
        }
    })
    .await
    .unwrap();
    println!("TCP {local}: {received} bytes; largest normalized batch {maximum_batch}");
    if tun.capabilities().vnet_header {
        assert!(maximum_batch > 1);
    }
    let packets: Vec<_> = (0..3)
        .map(|index| {
            tcp_reply(
                remote,
                local,
                port,
                sequence.wrapping_add(8193),
                false,
                (index * 1000, &[0x6b; 1000]),
            )
        })
        .collect();
    let refs: Vec<_> = packets.iter().map(Vec::as_slice).collect();
    let sent = tun.send_batch(&refs).await.unwrap();
    assert_eq!(sent, if tun.capabilities().tso4 { 3 } else { 1 });
    for packet in &refs[sent..] {
        tun.send(packet).await.unwrap();
    }
    timeout(WAIT, delivery).await.unwrap().unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

fn tcp_reply(
    source: IpAddr,
    destination: IpAddr,
    port: u16,
    ack: u32,
    syn: bool,
    data: (u32, &[u8]),
) -> Vec<u8> {
    // Start with the shared fixture's IP header, replacing UDP with TCP.
    let ip = if source.is_ipv4() { 20 } else { 40 };
    let mut packet = udp_packet(source, destination, 42424, port, &[]);
    packet.resize(ip + if syn { 24 } else { 20 }, 0);
    packet[ip..].fill(0);
    packet[ip..ip + 2].copy_from_slice(&42424_u16.to_be_bytes());
    packet[ip + 2..ip + 4].copy_from_slice(&port.to_be_bytes());
    packet[ip + 4..ip + 8]
        .copy_from_slice(&(if syn { 777_u32 } else { 778 + data.0 }).to_be_bytes());
    packet[ip + 8..ip + 12].copy_from_slice(&ack.to_be_bytes());
    packet[ip + 12] = if syn { 0x60 } else { 0x50 };
    packet[ip + 13] = if syn { 0x12 } else { 0x10 };
    packet[ip + 14..ip + 16].copy_from_slice(&65535_u16.to_be_bytes());
    if syn {
        packet[ip + 20..ip + 24].copy_from_slice(&[2, 4, 0x05, 0xb4]);
    }
    packet.extend_from_slice(data.1);
    let len = packet.len();
    let mut pseudo = Vec::new();
    if source.is_ipv4() {
        packet[9] = 6;
        packet[4..6].copy_from_slice(&((data.0 / 1000) as u16).to_be_bytes());
        packet[2..4].copy_from_slice(&(len as u16).to_be_bytes());
        packet[10..12].fill(0);
        let value = checksum(&packet[..ip]);
        packet[10..12].copy_from_slice(&value.to_be_bytes());
        pseudo.extend_from_slice(&packet[12..20]);
        pseudo.extend_from_slice(&[0, 6]);
        pseudo.extend_from_slice(&((len - ip) as u16).to_be_bytes());
    } else {
        packet[6] = 6;
        packet[4..6].copy_from_slice(&((len - ip) as u16).to_be_bytes());
        pseudo.extend_from_slice(&packet[8..40]);
        pseudo.extend_from_slice(&((len - ip) as u32).to_be_bytes());
        pseudo.extend_from_slice(&[0, 0, 0, 6]);
    }
    pseudo.extend_from_slice(&packet[ip..]);
    let value = checksum(&pseudo);
    packet[ip + 16..ip + 18].copy_from_slice(&value.to_be_bytes());
    packet
}

#[tokio::test]
#[ignore = "requires explicit isolated network namespace, /dev/net/tun, NET_ADMIN, and MESHWAY_TUN_TEST_ISOLATED=1; see file header"]
async fn owned_devices_ipv4_ipv6_packet_io() {
    assert_eq!(
        std::env::var("MESHWAY_TUN_TEST_ISOLATED").as_deref(),
        Ok("1")
    );
    let initial = interfaces();
    assert_eq!(
        initial,
        BTreeSet::from([String::from("lo")]),
        "use --network none in a fresh container"
    );

    // This first queue represents another owner. Creating, rejecting options,
    // using, and dropping the second device must preserve it and its MTU/index.
    let other = Tun::create(Options::new(1400).unwrap()).unwrap();
    let other_name = other.name().to_owned();
    let other_index = attribute(&other_name, "ifindex");
    assert_eq!(attribute(&other_name, "mtu").trim(), "1400");
    let before_failure = interfaces();
    assert!(matches!(Options::new(0), Err(Error::InvalidMtu { .. })));
    assert!(matches!(
        Options::new(MTU).unwrap().with_ipv4(LOCAL_V4, 33),
        Err(Error::InvalidPrefix { .. })
    ));
    assert_eq!(interfaces(), before_failure);

    // A create attempt with no entered reactor must fail without allocating or
    // changing any device, even while another owner has a live TUN.
    let missing_runtime = std::thread::spawn(|| {
        matches!(
            Tun::create(Options::new(MTU).unwrap()),
            Err(Error::NoRuntime)
        )
    })
    .join()
    .unwrap();
    assert!(missing_runtime);
    assert_eq!(interfaces(), before_failure);

    // Linux rejects a multicast interface address after tun-rs has allocated
    // the new queue. This exercises rollback of a real configuration failure,
    // rather than only userspace option validation.
    let bad_address = Options::new(MTU)
        .unwrap()
        .with_ipv6("ff02::1".parse().unwrap(), 64)
        .unwrap();
    assert!(matches!(Tun::create(bad_address), Err(Error::Io(_))));
    timeout(WAIT, async {
        while interfaces() != before_failure {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("failed creation leaked its newly allocated interface");
    assert_eq!(attribute(&other_name, "ifindex"), other_index);
    assert_eq!(attribute(&other_name, "mtu").trim(), "1400");

    let options = Options::new(MTU)
        .unwrap()
        .with_ipv4(LOCAL_V4, 30)
        .unwrap()
        .with_ipv6(LOCAL_V6, 126)
        .unwrap();
    let tun = Tun::create(options).unwrap();
    let name = tun.name().to_owned();
    assert_ne!(name, other_name);
    assert_ne!(attribute(&name, "ifindex"), other_index);
    assert_eq!(tun.mtu(), MTU);
    assert_eq!(attribute(&name, "mtu").trim(), "1500");
    let flags = attribute(&name, "tun_flags");
    let flags = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).unwrap();
    assert_eq!(flags & 0x1001, 0x1001, "IFF_TUN | IFF_NO_PI");
    assert_eq!(
        flags & (0x0002 | 0x0800 | 0x0100 | 0x4000),
        0,
        "no TAP, persistence, multiqueue, or vnet header"
    );

    wait_for_ipv6(&name).await;
    assert!(matches!(
        tun.send(&[]).await,
        Err(Error::InvalidPacketSize { .. })
    ));
    assert!(matches!(
        tun.send(&vec![0; MTU + 1]).await,
        Err(Error::InvalidPacketSize { .. })
    ));

    // Drain only the bounded startup control traffic (e.g. IPv6 DAD). Once the
    // queue is empty, poll a receive to Pending and actually drop that future.
    // The following IPv4 and IPv6 exchanges prove the same queue remains usable.
    cancel_empty_receive(&tun).await;
    exchange(&tun, LOCAL_V4.into(), REMOTE_V4.into()).await;
    exchange(&tun, LOCAL_V6.into(), REMOTE_V6.into()).await;

    drop(tun);
    timeout(WAIT, async {
        while interfaces().contains(&name) {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropped nonpersistent interface remained present");
    assert_eq!(interfaces(), before_failure);
    assert_eq!(attribute(&other_name, "ifindex"), other_index);
    assert_eq!(attribute(&other_name, "mtu").trim(), "1400");
    drop(other);
    timeout(WAIT, async {
        while interfaces() != initial {
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remaining owned interface was not removed on drop");

    // The locked test:tun:linux runner selects this exact lifecycle test.
    // Keep the offload regressions on that real isolated-kernel path too.
    upstream::verify_codec();
    verify_offload().await;
    assert_eq!(interfaces(), initial);
}

fn interfaces() -> BTreeSet<String> {
    std::fs::read_to_string("/proc/self/net/dev")
        .unwrap()
        .lines()
        .skip(2)
        .map(|line| line.split_once(':').unwrap().0.trim().to_owned())
        .collect()
}

fn attribute(name: &str, attribute: &str) -> String {
    std::fs::read_to_string(format!("/sys/class/net/{name}/{attribute}")).unwrap()
}

async fn wait_for_ipv6(name: &str) {
    let address = format!("{:032x}", u128::from(LOCAL_V6));
    timeout(WAIT, async {
        loop {
            let addresses = std::fs::read_to_string("/proc/self/net/if_inet6").unwrap();
            if addresses.lines().any(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                fields.len() == 6
                    && fields[0] == address
                    && fields[5] == name
                    && u32::from_str_radix(fields[4], 16).unwrap() & (0x40 | 0x08) == 0
            }) {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("IPv6 address did not become usable (tentative/dadfailed/disabled)");
}

async fn cancel_empty_receive(tun: &Tun) {
    let mut buffer = vec![0; MTU];
    timeout(WAIT, async {
        while let Ok(result) = timeout(Duration::from_millis(30), tun.recv(&mut buffer)).await {
            result.unwrap();
        }
    })
    .await
    .expect("startup traffic never quiesced");
    let mut pending = Box::pin(tun.recv(&mut buffer));
    poll_fn(|cx| {
        assert!(
            pending.as_mut().poll(cx).is_pending(),
            "expected an empty TUN queue"
        );
        Poll::Ready(())
    })
    .await;
    drop(pending);
}

async fn exchange(tun: &Tun, local: IpAddr, remote: IpAddr) {
    let socket = UdpSocket::bind(SocketAddr::new(local, 0)).await.unwrap();
    let local_port = socket.local_addr().unwrap().port();
    let remote_port = 42424;
    socket
        .connect(SocketAddr::new(remote, remote_port))
        .await
        .unwrap();

    // Queue a maximum-MTU datagram followed by a short datagram to expose
    // truncation, merging, and stream-style reads. Reject a small buffer while
    // both datagrams are queued, then verify their boundaries and order.
    let header_len = if local.is_ipv4() { 28 } else { 48 };
    let payloads = [vec![0x5a; MTU - header_len], vec![0xa5; 73]];
    for payload in &payloads {
        assert_eq!(socket.send(payload).await.unwrap(), payload.len());
    }
    assert!(matches!(
        tun.recv(&mut [0; 1]).await,
        Err(Error::BufferTooSmall {
            required: MTU,
            actual: 1
        })
    ));
    for payload in payloads {
        let mut packet = vec![0; MTU];
        timeout(WAIT, async {
            loop {
                let len = tun.recv(&mut packet).await.unwrap();
                if fixture_udp_payload(&packet[..len], local, remote, local_port, remote_port)
                    == Some(payload.as_slice())
                {
                    assert_eq!(len, header_len + payload.len());
                    upstream::assert_checksums(&packet[..len]);
                    break;
                }
            }
        })
        .await
        .expect("OS UDP payload did not arrive as a complete raw IP datagram");

        let reply = udp_packet(remote, local, remote_port, local_port, &payload);
        assert_eq!(reply.len(), header_len + payload.len());
        timeout(WAIT, tun.send(&reply)).await.unwrap().unwrap();
        let mut delivered = vec![0; MTU];
        let len = timeout(WAIT, socket.recv(&mut delivered))
            .await
            .expect("kernel did not deliver the injected UDP payload")
            .unwrap();
        assert_eq!(&delivered[..len], payload);
    }
}

// Packet construction/inspection is deliberately confined to this kernel test
// fixture. Production packet parsing and policy belong to meshway-dataplane.
fn fixture_udp_payload(
    packet: &[u8],
    source: IpAddr,
    destination: IpAddr,
    source_port: u16,
    destination_port: u16,
) -> Option<&[u8]> {
    let header = match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            if packet.len() < 28
                || packet[0] != 0x45
                || packet[9] != 17
                || packet[12..16] != source.octets()
                || packet[16..20] != destination.octets()
                || usize::from(u16::from_be_bytes([packet[2], packet[3]])) != packet.len()
            {
                return None;
            }
            20
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            if packet.len() < 48
                || packet[0] >> 4 != 6
                || packet[6] != 17
                || packet[8..24] != source.octets()
                || packet[24..40] != destination.octets()
                || usize::from(u16::from_be_bytes([packet[4], packet[5]])) + 40 != packet.len()
            {
                return None;
            }
            40
        }
        _ => return None,
    };
    if packet[header..header + 2] != source_port.to_be_bytes()
        || packet[header + 2..header + 4] != destination_port.to_be_bytes()
        || usize::from(u16::from_be_bytes([packet[header + 4], packet[header + 5]]))
            != packet.len() - header
    {
        return None;
    }
    Some(&packet[header + 8..])
}

fn udp_packet(
    source: IpAddr,
    destination: IpAddr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let udp_len = u16::try_from(8 + payload.len()).unwrap();
    let header = if source.is_ipv4() { 20 } else { 40 };
    let mut packet = vec![0; header + usize::from(udp_len)];
    let mut pseudo = Vec::new();
    match (source, destination) {
        (IpAddr::V4(source), IpAddr::V4(destination)) => {
            packet[0] = 0x45;
            let len = u16::try_from(packet.len()).unwrap();
            packet[2..4].copy_from_slice(&len.to_be_bytes());
            packet[8] = 64;
            packet[9] = 17;
            packet[12..16].copy_from_slice(&source.octets());
            packet[16..20].copy_from_slice(&destination.octets());
            let checksum = checksum(&packet[..20]);
            packet[10..12].copy_from_slice(&checksum.to_be_bytes());
            pseudo.extend_from_slice(&source.octets());
            pseudo.extend_from_slice(&destination.octets());
            pseudo.extend_from_slice(&[0, 17]);
            pseudo.extend_from_slice(&udp_len.to_be_bytes());
        }
        (IpAddr::V6(source), IpAddr::V6(destination)) => {
            packet[0] = 0x60;
            packet[4..6].copy_from_slice(&udp_len.to_be_bytes());
            packet[6] = 17;
            packet[7] = 64;
            packet[8..24].copy_from_slice(&source.octets());
            packet[24..40].copy_from_slice(&destination.octets());
            pseudo.extend_from_slice(&source.octets());
            pseudo.extend_from_slice(&destination.octets());
            pseudo.extend_from_slice(&u32::from(udp_len).to_be_bytes());
            pseudo.extend_from_slice(&[0, 0, 0, 17]);
        }
        _ => panic!("fixture address families differ"),
    }
    packet[header..header + 2].copy_from_slice(&source_port.to_be_bytes());
    packet[header + 2..header + 4].copy_from_slice(&destination_port.to_be_bytes());
    packet[header + 4..header + 6].copy_from_slice(&udp_len.to_be_bytes());
    packet[header + 8..].copy_from_slice(payload);
    pseudo.extend_from_slice(&packet[header..]);
    let checksum = match checksum(&pseudo) {
        0 => u16::MAX,
        checksum => checksum,
    };
    packet[header + 6..header + 8].copy_from_slice(&checksum.to_be_bytes());
    packet
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0_u32;
    for pair in bytes.chunks(2) {
        sum += u32::from(u16::from_be_bytes([
            pair[0],
            pair.get(1).copied().unwrap_or(0),
        ]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !u16::try_from(sum).unwrap()
}
