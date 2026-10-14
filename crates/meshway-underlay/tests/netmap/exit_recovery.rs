//! OPS8: the exit role's own failures stay the exit role's. An echo
//! connection whose conntrack entry ended while earlier projections lived is
//! re-created by a later sequence; retiring the earlier ones is no ownership
//! conflict. A deleted exit-owned route stops only the role: it cleans up
//! exactly, the Host keeps its capture and protection, and the role installs
//! again on the same content.
use super::{cutoff, host, isolated, routable};
use crate::support::{HOUR, Network, Server, listener, sync};
use meshway_underlay::{
    CtConfirmation, ExitIncarnation, ExitRole, ExitScope, ExitSendGuard, ExitTransport,
    Invalidated, OriginalTuple,
};
use std::{net::Ipv4Addr, process::Command, time::Duration};
use tokio_util::sync::CancellationToken;

const SOURCE: Ipv4Addr = Ipv4Addr::new(100, 64, 0, 2);
const IDENTIFIER: u16 = 4343;

struct Open;
impl ExitSendGuard for Open {
    fn check(&self) -> Result<(), Invalidated> {
        Ok(())
    }
}

fn incarnation(sequence: u16) -> ExitIncarnation {
    let mut id = [0u8; 32];
    id[0] = 0xe8;
    id[1..3].copy_from_slice(&sequence.to_be_bytes());
    ExitIncarnation::new(id)
}

fn echo(sequence: u16) -> OriginalTuple {
    OriginalTuple {
        source: SOURCE,
        destination: routable(),
        transport: ExitTransport::IcmpEcho {
            identifier: IDENTIFIER,
            sequence,
        },
    }
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = bytes
        .chunks(2)
        .map(|pair| u32::from(u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)])))
        .sum::<u32>();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// One IPv4 ICMP echo request of the source, as the exit TUN receives it.
fn request(sequence: u16) -> Vec<u8> {
    let mut icmp = vec![8, 0, 0, 0];
    icmp.extend_from_slice(&IDENTIFIER.to_be_bytes());
    icmp.extend_from_slice(&sequence.to_be_bytes());
    icmp.extend_from_slice(b"meshway-x8-echo!");
    let sum = checksum(&icmp);
    icmp[2..4].copy_from_slice(&sum.to_be_bytes());
    let length = u16::try_from(20 + icmp.len()).unwrap();
    let mut packet = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, 1, 0, 0];
    packet[2..4].copy_from_slice(&length.to_be_bytes());
    packet.extend_from_slice(&SOURCE.octets());
    packet.extend_from_slice(&routable().octets());
    let sum = checksum(&packet);
    packet[10..12].copy_from_slice(&sum.to_be_bytes());
    packet.extend_from_slice(&icmp);
    packet
}

/// Install, send and confirm one echo projection; its conntrack id.
async fn exchange(role: &ExitRole, scope: &ExitScope, sequence: u16) -> u32 {
    role.install(
        incarnation(sequence),
        echo(sequence),
        cutoff(120_000),
        scope.clone(),
    )
    .await
    .unwrap_or_else(|error| panic!("echo {sequence} installs: {error}"));
    role.send(incarnation(sequence), request(sequence), Open)
        .await
        .unwrap();
    match role.confirm(incarnation(sequence)).await.unwrap() {
        CtConfirmation::Confirmed { conntrack_id, .. } => conntrack_id,
        other => panic!("echo {sequence} not confirmed: {other:?}"),
    }
}

fn tool(program: &str, arguments: &[&str]) -> String {
    let output = Command::new(program).args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "{program} {arguments:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

/// End the source's echo conntrack entry, as its ICMP timeout does. The
/// kernel derives an entry's id from its slot; local flows take freed slots
/// first, so the next entry of the same echo id usually has a new id. The
/// returned sockets keep those flows' entries alive.
fn end_entry() -> Vec<std::net::UdpSocket> {
    tool(
        "conntrack",
        &["-D", "-f", "ipv4", "-p", "icmp", "-s", &SOURCE.to_string()],
    );
    let remote = std::net::SocketAddr::from((routable(), 18080));
    (0..16)
        .map(|_| {
            let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
            socket.send_to(b"x8-slot", remote).unwrap();
            socket
        })
        .collect()
}

/// The exit role's return table: the one of its pair rule for this source.
fn return_table() -> String {
    let rules: Vec<serde_json::Value> =
        serde_json::from_str(&tool("ip", &["-j", "-4", "rule"])).unwrap();
    let source = SOURCE.to_string();
    let remote = routable().to_string();
    rules
        .iter()
        .find(|rule| rule["src"] == remote.as_str() && rule["dst"] == source.as_str())
        .and_then(|rule| rule["table"].as_str().map(str::to_owned))
        .expect("pair rule of the source")
}

async fn ends(role: &ExitRole) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while role.is_active() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("exit role ends");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an isolated Linux exit namespace (ip_forward=1, loose RPF), nft, conntrack and NET_ADMIN"]
async fn netmap_exit_echo_reconnect_and_route_loss_stop_only_the_role() {
    let temp = isolated();
    let server = Server::start().await;
    let (mut control, script) = server.enroll(temp.path()).await;
    let mut network = Network::new(&control, true);
    let first = network.compile(None);
    let role_plan = first.netmap.exit_role.clone().expect("exit role");
    let snapshot = sync(&mut control, &script, &first, true, HOUR).await;
    let mut host = host(&temp).await;
    host.install(snapshot).await.unwrap();
    let role = host.install_netmap_exit_role(&role_plan).await.unwrap();
    let scope = ExitScope {
        binding: meshway_model::Digest::from_bytes([75; 32]),
        revoked: CancellationToken::new(),
    };

    // The connection's entry ends while sequence 1 is still projected (its
    // ICMP timeout, here a deletion); sequence 2 creates the next entry.
    let old = exchange(&role, &scope, 1).await;
    // A re-created entry that took the same slot has the same id; end it
    // again until the id differs, as after the 17:47 incident.
    let mut slots = Vec::new();
    let mut sequence = 1;
    let current = loop {
        slots.extend(end_entry());
        sequence += 1;
        let current = exchange(&role, &scope, sequence).await;
        if current != old || sequence == 6 {
            break current;
        }
    };
    assert_ne!(old, current, "a new conntrack entry for the same echo id");
    // A later sequence joins the current entry; the stale one is no owner.
    let joined = exchange(&role, &scope, 10).await;
    assert_eq!(joined, current);
    // Retiring sequence 1 (its entry long gone) keeps the current one.
    role.retire_flow(incarnation(1)).await.unwrap();
    assert!(role.is_active(), "a replaced echo entry is no conflict");
    // Sequence 10 stays projected: its pair rule names the return table below.
    for retired in 2..=sequence {
        role.retire_flow(incarnation(retired)).await.unwrap();
    }
    assert!(role.is_active());
    drop(slots);
    println!("X8_EXIT_ECHO_RECONNECT=old={old},current={current},role-kept");

    // Someone deletes the source's return route: only the role stops, and
    // its exact cleanup completes.
    let table = return_table();
    tool(
        "ip",
        &["route", "del", &format!("{SOURCE}/32"), "table", &table],
    );
    ends(&role).await;
    role.retire()
        .await
        .expect("exact cleanup of the stopped role");
    assert!(host.exit_role().is_none());
    // The Host's own resources are untouched: a new serial applies.
    network.listeners.insert("udp".into(), listener(41004));
    let second = network.compile(Some(&first));
    let next = sync(&mut control, &script, &second, true, HOUR).await;
    host.install(next)
        .await
        .expect("Host protection survives the exit role's failure");
    // And the role installs again on the same content.
    let again = host.install_netmap_exit_role(&role_plan).await.unwrap();
    let renewed = exchange(&again, &scope, 4).await;
    again.retire_flow(incarnation(4)).await.unwrap();
    assert!(again.is_active());
    println!("X8_EXIT_ROUTE_LOSS=role-stopped,cleanup-exact,host-kept,reinstalled,ct={renewed}");

    host.suspend().await.unwrap();
    ends(&again).await;
    again.retire().await.unwrap();
    let _ = control.shutdown().await;
    host.shutdown(false).await.unwrap();
    server.close().await;
}
