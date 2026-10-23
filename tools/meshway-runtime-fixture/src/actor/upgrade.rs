//! `udp/off` carrier return (CM-1b): the A–R edge orders Native UDP before
//! Native TCP (R also listens on TCP, only A dials it). A drops its own
//! outgoing UDP, so A–R falls back to Native TCP; after the block is lifted
//! the edge must return to Native UDP within the candidate back-off ceiling
//! (60 s) plus a handshake, without restart, while the circuit carried on
//! TCP keeps its CID and complete path across the return.
use super::{Cli, Result, Role, Signals, Traffic, command, save, tool};
use meshway_model::{NodeId, TransportProtocol};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// The fallback itself: UDP health window (6 s) plus the UDP attempts and a
/// TCP connection.
const FALLBACK_LIMIT: Duration = Duration::from_secs(30);
/// Time on TCP before the block is lifted: failed attempts beside it.
const HOLD: Duration = Duration::from_secs(5);
/// The gate: the back-off ceiling (60 s), a handshake and the Status poll.
pub(super) const RETURN_LIMIT: Duration = Duration::from_secs(75);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence {
    device: String,
    blocked_at_ms: u64,
    tcp_at_ms: u64,
    unblocked_at_ms: u64,
    udp_at_ms: u64,
    return_ms: u64,
    original_path: Vec<NodeId>,
    tcp_circuits: BTreeSet<String>,
    udp_circuits: BTreeSet<String>,
    tcp_endpoint: String,
    udp_endpoint: String,
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

/// R's transport and endpoint in A's Status.
async fn carrier(cli: &Cli, relay: NodeId) -> Result<(Option<TransportProtocol>, String)> {
    let status = command(cli, "status").await?;
    let peer = status
        .peers
        .iter()
        .find(|peer| peer.node_id == relay)
        .ok_or("A's Status has no R")?;
    Ok((
        peer.transport,
        peer.endpoint
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    ))
}

/// Probe until R's carrier is `protocol`, within `limit`.
async fn until(
    cli: &Cli,
    traffic: &mut Traffic,
    relay: NodeId,
    protocol: TransportProtocol,
    limit: Duration,
) -> Result<String> {
    let deadline = Instant::now() + crate::remaining(limit)?;
    loop {
        traffic.probe().await?;
        if let (Some(actual), endpoint) = carrier(cli, relay).await?
            && actual == protocol
        {
            return Ok(endpoint);
        }
        if Instant::now() >= deadline {
            return Err(format!("A–R did not reach {protocol:?} within {limit:?}").into());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn checked(program: &str, args: &[&str]) -> Result<()> {
    let output = tool(program, args).await?;
    if !output.status.success() {
        return Err(format!(
            "{program} {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

/// Drop every outgoing UDP datagram of this namespace on `device`; tc is
/// not owned or audited by the Node (unlike its nftables tables).
async fn block(device: &str) -> Result<()> {
    checked("tc", &["qdisc", "add", "dev", device, "clsact"]).await?;
    for (protocol, matcher) in [("ip", "ip"), ("ipv6", "ip6")] {
        checked(
            "tc",
            &[
                "filter", "add", "dev", device, "egress", "protocol", protocol, "u32", "match",
                matcher, "protocol", "17", "0xff", "action", "drop",
            ],
        )
        .await?;
    }
    Ok(())
}

async fn unblock(device: &str) -> Result<()> {
    checked("tc", &["qdisc", "del", "dev", device, "clsact"]).await
}

/// CIDs of real business packets on the original path observed in
/// `[from, to)`; another path fails.
async fn circuits(cli: &Cli, path: &[NodeId], from: u64, to: u64) -> Result<BTreeSet<String>> {
    let mut cids = BTreeSet::new();
    for event in super::log_events(cli).await? {
        let Some(packet) = crate::fallback::packet_observation(&event)? else {
            continue;
        };
        if packet.observed_at_ms < from || packet.observed_at_ms >= to {
            continue;
        }
        let observed: Vec<NodeId> = serde_json::from_str(&packet.path_json)?;
        if observed != path {
            return Err("the carrier return changed the complete circuit path".into());
        }
        cids.insert(packet.cid.to_ascii_lowercase());
    }
    Ok(cids)
}

pub(super) async fn run(cli: &Cli, signals: &Signals, traffic: &mut Traffic) -> Result<()> {
    let path = crate::fallback::original_path(signals).await?;
    let relay = path[Role::ALL
        .iter()
        .position(|role| *role == Role::R)
        .ok_or("no R")?];
    traffic.exchange().await?;
    let (initial, endpoint) = carrier(cli, relay).await?;
    if initial != Some(TransportProtocol::NativeUdp) {
        return Err(format!("A–R starts on {initial:?}, not Native UDP").into());
    }
    let address = endpoint
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "A–R endpoint is not an address")?;
    let route = tool("ip", &["-o", "route", "get", &address.ip().to_string()]).await?;
    let route = String::from_utf8(route.stdout)?;
    let words: Vec<_> = route.split_whitespace().collect();
    let device = words
        .windows(2)
        .find(|pair| pair[0] == "dev")
        .map(|pair| pair[1].to_owned())
        .ok_or("no route device toward R")?;
    tracing::info!(phase = "carrier-upgrade-block", %device, "A drops its outgoing UDP");
    block(&device).await?;
    let blocked_at_ms = now_ms()?;
    let result = async {
        let tcp_endpoint = until(
            cli,
            traffic,
            relay,
            TransportProtocol::NativeTcp,
            FALLBACK_LIMIT,
        )
        .await?;
        let tcp_at_ms = now_ms()?;
        tracing::info!(phase = "carrier-upgrade-tcp", %tcp_endpoint, "A–R fell back to Native TCP");
        // Business on the TCP carrier; the attempts beside it keep failing.
        traffic.exchange().await?;
        let started = Instant::now();
        while started.elapsed() < HOLD {
            traffic.probe().await?;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        traffic.exchange().await?;
        Ok::<_, crate::Error>((tcp_endpoint, tcp_at_ms))
    }
    .await;
    // The block never outlives the stage, whatever its outcome.
    unblock(&device).await?;
    let (tcp_endpoint, tcp_at_ms) = result?;
    let unblocked_at_ms = now_ms()?;
    tracing::info!(
        phase = "carrier-upgrade-unblock",
        "A's outgoing UDP is lifted"
    );
    let udp_endpoint = until(
        cli,
        traffic,
        relay,
        TransportProtocol::NativeUdp,
        RETURN_LIMIT,
    )
    .await?;
    let udp_at_ms = now_ms()?;
    traffic.exchange().await?;
    let after_ms = now_ms()?.saturating_add(1);
    let tcp_circuits = circuits(cli, &path, tcp_at_ms, unblocked_at_ms).await?;
    let udp_circuits = circuits(cli, &path, udp_at_ms, after_ms).await?;
    if tcp_circuits.is_empty() || tcp_circuits != udp_circuits {
        return Err(format!(
            "the circuit on TCP {tcp_circuits:?} was not kept across the return to UDP {udp_circuits:?}"
        )
        .into());
    }
    let evidence = Evidence {
        device,
        blocked_at_ms,
        tcp_at_ms,
        unblocked_at_ms,
        udp_at_ms,
        return_ms: udp_at_ms - unblocked_at_ms,
        original_path: path,
        tcp_circuits,
        udp_circuits,
        tcp_endpoint,
        udp_endpoint,
    };
    tracing::info!(
        phase = "carrier-upgrade-udp",
        return_ms = evidence.return_ms,
        "A–R returned to Native UDP with the same circuit"
    );
    save(cli, "carrier-upgrade", &evidence).await
}
