//! Real loopback UDP observers emitting RFC wire packets; no external network.
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use meshway_agent::netcheck::{AddressFamily, Config, Outcome, Report, Source, measure};
use tokio::{net::UdpSocket, task::JoinHandle};

const COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];

#[derive(Clone)]
enum Reply {
    Mapped(SocketAddr, bool),
    Mutated(fn(&mut Vec<u8>)),
    NoiseThenValid,
    WrongSourceOnly,
    WrongTransactionOnly,
    Flood,
    Silent,
    Error,
}

type SeenRequests = Arc<Mutex<Vec<(SocketAddr, [u8; 12])>>>;

struct Observer {
    address: SocketAddr,
    requests: SeenRequests,
    task: JoinHandle<()>,
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Observer {
    async fn bind(bind: &str, replies: Vec<Reply>) -> std::io::Result<Self> {
        let socket = UdpSocket::bind(bind).await?;
        let address = socket.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = requests.clone();
        let task = tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let mut count = 0;
            loop {
                let (len, peer) = socket.recv_from(&mut buffer).await.unwrap();
                assert_eq!(len, 20);
                assert_eq!(&buffer[..8], &[0, 1, 0, 0, 0x21, 0x12, 0xa4, 0x42]);
                let transaction: [u8; 12] = buffer[8..20].try_into().unwrap();
                seen.lock().unwrap().push((peer, transaction));
                let reply = replies
                    .get(count)
                    .unwrap_or_else(|| replies.last().unwrap())
                    .clone();
                count += 1;
                let mut packet = mapped(transaction, peer, true);
                match reply {
                    Reply::Mapped(endpoint, xor) => packet = mapped(transaction, endpoint, xor),
                    Reply::Mutated(mutate) => mutate(&mut packet),
                    Reply::Error => {
                        packet = message(transaction, 0x0111, &[(0x0009, vec![0, 0, 4, 1])])
                    }
                    Reply::Silent => continue,
                    Reply::WrongSourceOnly => {
                        let rogue = UdpSocket::bind(if address.is_ipv4() {
                            "127.0.0.1:0"
                        } else {
                            "[::1]:0"
                        })
                        .await
                        .unwrap();
                        rogue.send_to(&packet, peer).await.unwrap();
                        continue;
                    }
                    Reply::WrongTransactionOnly => {
                        packet[8] ^= 1;
                    }
                    Reply::Flood => {
                        // Wrong transaction from the expected peer must never monopolize the executor.
                        packet[8] ^= 1;
                        let until = tokio::time::Instant::now() + Duration::from_millis(450);
                        while tokio::time::Instant::now() < until {
                            for _ in 0..32 {
                                socket.send_to(&packet, peer).await.unwrap();
                            }
                            tokio::task::yield_now().await;
                        }
                        continue;
                    }
                    Reply::NoiseThenValid => {
                        let rogue = UdpSocket::bind("127.0.0.1:0").await.unwrap();
                        rogue.send_to(&packet, peer).await.unwrap();
                        let mut wrong = packet.clone();
                        wrong[8] ^= 1;
                        socket.send_to(&wrong, peer).await.unwrap();
                        wrong = packet.clone();
                        wrong[4] ^= 1;
                        socket.send_to(&wrong, peer).await.unwrap();
                        wrong = packet.clone();
                        wrong[0] = 0x41;
                        socket.send_to(&wrong, peer).await.unwrap();
                        wrong = packet.clone();
                        wrong[1] = 0x02;
                        socket.send_to(&wrong, peer).await.unwrap();
                        wrong = packet.clone();
                        wrong[3] = 0;
                        socket.send_to(&wrong, peer).await.unwrap();
                    }
                }
                socket.send_to(&packet, peer).await.unwrap();
            }
        });
        Ok(Self {
            address,
            requests,
            task,
        })
    }
}

fn message(transaction: [u8; 12], kind: u16, attrs: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut packet = Vec::from(kind.to_be_bytes());
    packet.extend([0, 0]);
    packet.extend(COOKIE);
    packet.extend(transaction);
    for (kind, value) in attrs {
        packet.extend(kind.to_be_bytes());
        packet.extend(u16::try_from(value.len()).unwrap().to_be_bytes());
        packet.extend(value);
        while packet.len() % 4 != 0 {
            packet.push(0);
        }
    }
    let len = u16::try_from(packet.len() - 20).unwrap();
    packet[2..4].copy_from_slice(&len.to_be_bytes());
    packet
}

fn mapped(transaction: [u8; 12], endpoint: SocketAddr, xor: bool) -> Vec<u8> {
    let mut value = vec![0, if endpoint.is_ipv4() { 1 } else { 2 }];
    value.extend((endpoint.port() ^ if xor { 0x2112 } else { 0 }).to_be_bytes());
    let octets = match endpoint.ip() {
        std::net::IpAddr::V4(ip) => ip.octets().to_vec(),
        std::net::IpAddr::V6(ip) => ip.octets().to_vec(),
    };
    for (i, byte) in octets.into_iter().enumerate() {
        let mask = if !xor {
            0
        } else if i < 4 {
            COOKIE[i]
        } else {
            transaction[i - 4]
        };
        value.push(byte ^ mask);
    }
    message(
        transaction,
        0x0101,
        &[(if xor { 0x0020 } else { 0x0001 }, value)],
    )
}

fn set_length(packet: &mut [u8]) {
    let len = u16::try_from(packet.len() - 20).unwrap();
    packet[2..4].copy_from_slice(&len.to_be_bytes());
}

fn fingerprint(packet: &mut Vec<u8>) {
    packet.extend([0x80, 0x28, 0, 4, 0, 0, 0, 0]);
    set_length(packet);
    let end = packet.len();
    // Independent fixture implementation (polynomial long division).
    let mut crc = !0_u32;
    for byte in &packet[..end - 8] {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    packet[end - 4..].copy_from_slice(&(!crc ^ 0x5354_554e).to_be_bytes());
}

async fn check(observers: &[&Observer]) -> Report {
    measure(
        Config::new(
            observers.iter().map(|o| o.address).collect(),
            if observers[0].address.is_ipv4() {
                "127.0.0.1:0"
            } else {
                "[::1]:0"
            }
            .parse()
            .unwrap(),
            250,
        )
        .unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn ipv4_xor_and_mapped_fallback_and_noise_recovery() {
    for xor in [true, false] {
        let endpoint = "198.51.100.9:54321".parse().unwrap();
        let observer = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(endpoint, xor)])
            .await
            .unwrap();
        let report = check(&[&observer]).await;
        assert_eq!(report.source, Source::IndependentUdpProbe);
        assert_eq!(report.address_family, AddressFamily::Ipv4);
        assert!(report.udp_observed);
        assert_eq!(
            report.samples[0].mapped_endpoint,
            Some(endpoint.to_string())
        );
        assert!(report.samples[0].rtt_ms.is_some());
        assert_eq!(report.mapping_stable_during_check, None);
        assert_eq!(report.mapping_varies_by_destination, None);
        assert!(report.is_fresh(Utc::now()));
    }
    let observer = Observer::bind("127.0.0.1:0", vec![Reply::NoiseThenValid])
        .await
        .unwrap();
    assert_eq!(
        check(&[&observer]).await.samples[0].outcome,
        Outcome::Mapped
    );
}

#[tokio::test]
async fn native_ipv6_xor_and_mapped_fallback() {
    for xor in [true, false] {
        let endpoint = "[2001:db8:1234:5678:abcd:ef01:2345:6789]:43210"
            .parse()
            .unwrap();
        let observer = match Observer::bind("[::1]:0", vec![Reply::Mapped(endpoint, xor)]).await {
            Ok(observer) => observer,
            Err(error)
                if error.kind() == std::io::ErrorKind::AddrNotAvailable
                    || matches!(error.raw_os_error(), Some(47 | 97)) =>
            {
                eprintln!("NoIpv6: native loopback unavailable: {error}");
                return;
            }
            Err(error) => panic!("unexpected IPv6 bind failure: {error}"),
        };
        let report = check(&[&observer]).await;
        assert_eq!(report.address_family, AddressFamily::Ipv6);
        assert_eq!(
            report.samples[0].mapped_endpoint,
            Some(endpoint.to_string())
        );
        let server = observer.address.to_string();
        let output = cli(&["--stun-server", &server, "--bind", "[::]:0", "--json"]).await;
        assert_eq!(output.status.code(), Some(0));
        let report: Report = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report.address_family, AddressFamily::Ipv6);
        for endpoint in [
            "[::]:12",
            "[ff02::1]:12",
            "[::ffff:192.0.2.1]:12",
            "[2001:db8::1]:0",
            "192.0.2.1:12",
        ] {
            let invalid = Observer::bind(
                "[::1]:0",
                vec![Reply::Mapped(endpoint.parse().unwrap(), true)],
            )
            .await
            .unwrap();
            assert_eq!(
                check(&[&invalid]).await.samples[0].outcome,
                Outcome::InvalidResponse,
                "{endpoint}"
            );
        }
    }
}

#[tokio::test]
async fn same_socket_new_transactions_order_and_same_ip_unknown() {
    let endpoint = "198.51.100.9:54321".parse().unwrap();
    let a = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(endpoint, true)])
        .await
        .unwrap();
    let b = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(endpoint, true)])
        .await
        .unwrap();
    let report = check(&[&a, &b]).await;
    assert_eq!(
        report
            .samples
            .iter()
            .map(|s| s.server.as_str())
            .collect::<Vec<_>>(),
        [
            a.address.to_string(),
            b.address.to_string(),
            a.address.to_string()
        ]
    );
    assert_eq!(report.mapping_stable_during_check, Some(true));
    assert_eq!(report.mapping_varies_by_destination, None);
    let a_requests = a.requests.lock().unwrap();
    let b_requests = b.requests.lock().unwrap();
    assert_eq!(a_requests.len(), 2);
    assert_eq!(b_requests.len(), 1);
    assert_eq!(a_requests[0].0, a_requests[1].0);
    assert_eq!(a_requests[0].0, b_requests[0].0);
    assert_eq!(report.bound_socket, a_requests[0].0.to_string());
    assert_ne!(a_requests[0].1, a_requests[1].1);
    assert_ne!(a_requests[0].1, b_requests[0].1);
    assert_ne!(a_requests[1].1, b_requests[0].1);
}

#[tokio::test]
async fn repeat_change_and_partial_observations_remain_conservative() {
    let one = "198.51.100.9:54321".parse().unwrap();
    let two = "198.51.100.9:54322".parse().unwrap();
    for (b_reply, final_a, stable, varies) in [
        (Reply::Mapped(one, true), one, Some(true), None),
        (Reply::Mapped(two, true), one, Some(true), None),
        (Reply::Mapped(two, true), two, Some(false), None),
        (Reply::Silent, one, Some(true), None),
    ] {
        let a = Observer::bind(
            "127.0.0.1:0",
            vec![Reply::Mapped(one, true), Reply::Mapped(final_a, true)],
        )
        .await
        .unwrap();
        let b = Observer::bind("127.0.0.1:0", vec![b_reply]).await.unwrap();
        let report = check(&[&a, &b]).await;
        assert_eq!(report.mapping_stable_during_check, stable);
        assert_eq!(report.mapping_varies_by_destination, varies);
    }
    let a = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(one, true), Reply::Silent])
        .await
        .unwrap();
    let b = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(one, true)])
        .await
        .unwrap();
    let report = check(&[&a, &b]).await;
    assert!(report.udp_observed);
    assert_eq!(report.mapping_stable_during_check, None);
    assert_eq!(report.mapping_varies_by_destination, None);
}

#[tokio::test]
async fn identity_timeout_and_flood_are_bounded_and_not_mapped() {
    for reply in [
        Reply::Silent,
        Reply::WrongSourceOnly,
        Reply::WrongTransactionOnly,
        Reply::Flood,
    ] {
        let observer = Observer::bind("127.0.0.1:0", vec![reply]).await.unwrap();
        let start = tokio::time::Instant::now();
        // Current-thread executor: timer must progress even under continuously readable UDP.
        let report = tokio::time::timeout(Duration::from_secs(2), check(&[&observer]))
            .await
            .unwrap();
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(report.samples[0].outcome, Outcome::Timeout);
        assert!(report.samples[0].elapsed_ms >= 240);
        assert!(!report.udp_observed);
        assert_eq!(report.samples[0].mapped_endpoint, None);
        assert_eq!(report.samples[0].rtt_ms, None);
    }
}

#[tokio::test]
async fn strict_bounds_duplicates_fingerprint_and_required_attributes() {
    let mutations: &[fn(&mut Vec<u8>)] = &[
        |p| p[3] -= 1, // non-four-aligned length
        |p| p[3] += 4, // length beyond packet
        |p| {
            p.extend([0; 4]);
        }, // trailing data
        |p| {
            p.pop();
        }, // truncated
        |p| {
            p[23] = 7;
        }, // wrong address attribute size
        |p| {
            p[23] = 100;
        }, // truncated attribute
        |p| {
            let attr = p[20..].to_vec();
            p.extend(attr);
            set_length(p);
        },
        |p| {
            fingerprint(p);
            let end = p.len();
            p[end - 1] ^= 1;
        },
        |p| {
            fingerprint(p);
            p.extend([0x80, 0x22, 0, 0]);
            set_length(p);
        },
        |p| {
            p.extend([0, 0x7f, 0, 0]);
            set_length(p);
        }, // unknown required
        |p| {
            p.extend([0, 8, 0, 20]);
            p.extend([0; 20]);
            set_length(p);
        }, // unsupported integrity
        |p| {
            p.resize(1028, 0);
            set_length(p);
        },
        |p| {
            // Valid 1024-byte prefix, with forged header hiding trailing datagram bytes.
            p.extend([0x80, 0xaa, 3, 0xdc]); // 988 bytes
            p.resize(1024, 0);
            set_length(p);
            p.extend([0; 4]);
        },
        |p| {
            for _ in 0..64 {
                p.extend([0x80, 0x22, 0, 0]);
            }
            set_length(p);
        },
        |p| {
            let mut attr = p[20..].to_vec();
            attr[1] = 1;
            attr[6] ^= 1;
            p.extend(attr);
            set_length(p);
        }, // contradictory fallback
        |p| {
            p[1] = 0x11;
        }, // error class with no ERROR-CODE
        |p| {
            p.truncate(19);
        }, // truncated header
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let observer = Observer::bind("127.0.0.1:0", vec![Reply::Mutated(*mutate)])
            .await
            .unwrap();
        let report = check(&[&observer]).await;
        assert_eq!(
            report.samples[0].outcome,
            Outcome::InvalidResponse,
            "mutation {index}"
        );
        assert!(!report.udp_observed);
    }
    for mutate in [
        fingerprint as fn(&mut Vec<u8>),
        |p: &mut Vec<u8>| {
            p.extend([0x80, 0xaa, 0, 1, 7, 0, 0, 0]);
            set_length(p);
        },
        |p: &mut Vec<u8>| {
            p.extend([0x80, 0xaa, 3, 0xdc]);
            p.resize(1024, 0);
            set_length(p);
        },
    ] {
        let observer = Observer::bind("127.0.0.1:0", vec![Reply::Mutated(mutate)])
            .await
            .unwrap();
        assert_eq!(
            check(&[&observer]).await.samples[0].outcome,
            Outcome::Mapped
        );
    }
    let observer = Observer::bind("127.0.0.1:0", vec![Reply::Error])
        .await
        .unwrap();
    assert_eq!(
        check(&[&observer]).await.samples[0].outcome,
        Outcome::StunError
    );
}

#[tokio::test]
async fn invalid_mapped_endpoints_are_rejected() {
    for endpoint in [
        "0.0.0.0:12",
        "224.0.0.1:12",
        "255.255.255.255:12",
        "198.51.100.1:0",
        "[2001:db8::1]:12",
        "[::ffff:192.0.2.1]:12",
    ] {
        let observer = Observer::bind(
            "127.0.0.1:0",
            vec![Reply::Mapped(endpoint.parse().unwrap(), true)],
        )
        .await
        .unwrap();
        assert_eq!(
            check(&[&observer]).await.samples[0].outcome,
            Outcome::InvalidResponse,
            "{endpoint}"
        );
    }
}

#[tokio::test]
async fn freshness_boundaries_and_closed_typed_json() {
    let observer = Observer::bind("127.0.0.1:0", vec![Reply::NoiseThenValid])
        .await
        .unwrap();
    let report = check(&[&observer]).await;
    assert!(report.is_fresh(report.completed_at));
    assert!(report.is_fresh(report.valid_until - chrono::Duration::nanoseconds(1)));
    assert!(!report.is_fresh(report.valid_until));
    assert!(!report.is_fresh(report.measured_at - chrono::Duration::nanoseconds(1)));
    let mut invalid = report.clone();
    invalid.valid_until += chrono::Duration::seconds(1);
    assert!(!invalid.is_fresh(report.completed_at));
    invalid = report.clone();
    invalid.completed_at = report.measured_at - chrono::Duration::seconds(1);
    assert!(!invalid.is_fresh(report.completed_at));
    invalid = report.clone();
    invalid.completed_at = report.measured_at + chrono::Duration::seconds(16);
    assert!(!invalid.is_fresh(invalid.completed_at));
    invalid = report.clone();
    invalid.schema_version = 2;
    assert!(!invalid.is_fresh(report.completed_at));
    invalid = report.clone();
    invalid.measured_at = chrono::DateTime::<Utc>::MAX_UTC;
    invalid.completed_at = invalid.measured_at;
    invalid.valid_until = invalid.measured_at;
    assert!(!invalid.is_fresh(invalid.measured_at));
    let json = serde_json::to_string(&report).unwrap();
    let _: Report = serde_json::from_str(&json).unwrap();
    for bad in [
        json.replacen('{', "{\"extra\":1,", 1),
        json.replace("\"server\":", "\"extra\":1,\"server\":"),
        json.replace("independent_udp_probe", "daemon"),
        json.replace("\"mapped\"", "\"healthy\""),
        json.replace("\"ipv4\"", "\"tcp\""),
        json.replace("\"schema_version\":1", "\"schema_version\":2"),
        json.replace("\"mapping_varies_by_destination\":null,", ""),
    ] {
        assert!(serde_json::from_str::<Report>(&bad).is_err());
    }
    let mut nullable = report;
    nullable.samples[0].rtt_ms = None;
    let missing = serde_json::to_string(&nullable)
        .unwrap()
        .replace("\"rtt_ms\":null,", "");
    assert!(serde_json::from_str::<Report>(&missing).is_err());
}

#[tokio::test]
async fn maximum_sequence_and_total_timeout_budget() {
    let mut observers = Vec::new();
    for _ in 0..4 {
        observers.push(
            Observer::bind("127.0.0.1:0", vec![Reply::Silent])
                .await
                .unwrap(),
        );
    }
    let start = tokio::time::Instant::now();
    let report = tokio::time::timeout(
        Duration::from_secs(3),
        check(&observers.iter().collect::<Vec<_>>()),
    )
    .await
    .unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
    assert_eq!(report.samples.len(), 5);
    assert!(
        report
            .samples
            .iter()
            .all(|sample| sample.outcome == Outcome::Timeout)
    );
    assert_eq!(report.samples[4].server, observers[0].address.to_string());
    assert_eq!(
        report.valid_until,
        report.measured_at + chrono::Duration::seconds(30)
    );
    assert_eq!(report.mapping_stable_during_check, None);
    assert_eq!(report.mapping_varies_by_destination, None);
    assert!(!report.udp_observed);
    let mut transactions = std::collections::HashSet::new();
    let bound: SocketAddr = report.bound_socket.parse().unwrap();
    for observer in &observers {
        for (peer, transaction) in observer.requests.lock().unwrap().iter() {
            assert_eq!(*peer, bound);
            assert!(transactions.insert(*transaction));
        }
    }
    assert_eq!(transactions.len(), 5);
}

async fn cli(args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .arg("netcheck")
        .args(args)
        .env("MESHWAY_STATE", "/unreadable/no-credentials.json")
        .output()
        .await
        .unwrap()
}

#[tokio::test]
async fn cli_usage_exit_json_human_partial_and_bind_conflict() {
    for args in [
        vec![],
        vec!["--stun-server", "localhost:3478"],
        vec!["--stun-server", "0.0.0.0:3478"],
        vec!["--stun-server", "224.0.0.1:3478"],
        vec!["--stun-server", "127.0.0.1:0"],
        vec!["--stun-server", "[fe80::1%2]:3478"],
        vec!["--stun-server", "[::1]:3478"],
        vec!["--stun-server", "127.0.0.1:3478", "--timeout-ms", "249"],
        vec!["--stun-server", "127.0.0.1:3478", "--timeout-ms", "3001"],
        vec![
            "--stun-server",
            "127.0.0.1:3478",
            "--stun-server",
            "127.0.0.1:3478",
        ],
        vec![
            "--stun-server",
            "127.0.0.1:1",
            "--stun-server",
            "127.0.0.1:2",
            "--stun-server",
            "127.0.0.1:3",
            "--stun-server",
            "127.0.0.1:4",
            "--stun-server",
            "127.0.0.1:5",
        ],
        vec!["--stun-server", "127.0.0.1:3478", "--bind", "224.0.0.1:0"],
        vec!["--stun-server", "127.0.0.1:3478", "--state", "private.json"],
    ] {
        let output = cli(&args).await;
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
    }
    let endpoint = "198.51.100.1:54321".parse().unwrap();
    let observer = Observer::bind("127.0.0.1:0", vec![Reply::Mapped(endpoint, true)])
        .await
        .unwrap();
    let server = observer.address.to_string();
    let output = cli(&[
        "--stun-server",
        &server,
        "--bind",
        "0.0.0.0:0",
        "--timeout-ms",
        "250",
        "--json",
    ])
    .await;
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.iter().filter(|b| **b == b'\n').count(), 1);
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report.samples[0].mapped_endpoint,
        Some(endpoint.to_string())
    );
    let human = cli(&["--stun-server", &server]).await;
    let human = String::from_utf8(human.stdout).unwrap();
    assert!(human.contains("Mapping varies by tested destination IPs: unknown"));
    assert!(human.contains("30s") && human.contains("socket is now closed"));
    assert!(human.contains("does not authenticate") && human.contains("IPv6 mesh"));
    let silent = Observer::bind("127.0.0.1:0", vec![Reply::Silent])
        .await
        .unwrap();
    let silent_server = silent.address.to_string();
    let output = cli(&[
        "--stun-server",
        &silent_server,
        "--timeout-ms",
        "250",
        "--json",
    ])
    .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.stderr.is_empty());
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!report.udp_observed);
    assert_eq!(report.samples[0].outcome, Outcome::Timeout);
    let output = cli(&[
        "--stun-server",
        &server,
        "--stun-server",
        &silent_server,
        "--timeout-ms",
        "250",
        "--json",
    ])
    .await;
    assert_eq!(output.status.code(), Some(0));
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert!(report.udp_observed);
    assert_eq!(report.mapping_varies_by_destination, None);
    let occupied = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let bind = occupied.local_addr().unwrap().to_string();
    let output = cli(&["--stun-server", &server, "--bind", &bind, "--json"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
    drop(occupied);
    let output = cli(&["--stun-server", &server, "--bind", &bind, "--json"]).await;
    assert_eq!(output.status.code(), Some(0));
    let report: Report = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report.bound_socket, bind);
}
