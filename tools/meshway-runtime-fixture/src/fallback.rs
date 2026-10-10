//! Same-application, same-circuit evidence from the real packet owner. Missing
//! production observations fail this optional case; none are synthesized here.
use crate::{
    Cli, Result, actor,
    application::{PacketEvidence, Traffic},
    signals::{Role, Signal, Signals},
};
use meshway_model::NodeId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    observed_at_ms: u64,
    packet: PacketEvidence,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub passed: bool,
    pub original_path: Vec<NodeId>,
    pub before_at_ms: u64,
    pub blocked_at_ms: u64,
    pub after_at_ms: u64,
    pub circuits: Vec<Circuit>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Circuit {
    cid: String,
    before_packets: usize,
    after_packets: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Blocked {
    blocked_at_ms: u64,
}

// Production trace seam: target meshway_daemon::runtime::network::circuit_packet,
// message "Mesh circuit packet ready". Emit only for a real business frame at
// the actual packet owner, using PendingFrame::circuit() and circuit_info().
// cid is 64 hex characters; path_json is a serialized Vec<NodeId>. No packet,
// credentials, cipher keys, substitute flow table or fixture callback is needed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PacketObservation {
    #[serde(rename = "message")]
    _message: String,
    pub cid: String,
    #[serde(rename = "target")]
    _target: String,
    pub path_json: String,
    pub observed_at_ms: u64,
}

pub(super) fn packet_observation(event: &serde_json::Value) -> Result<Option<PacketObservation>> {
    if event.get("target").and_then(serde_json::Value::as_str)
        != Some("meshway_daemon::runtime::network::circuit_packet")
        || event
            .get("fields")
            .and_then(|fields| fields.get("message"))
            .and_then(serde_json::Value::as_str)
            != Some("Mesh circuit packet ready")
    {
        return Ok(None);
    }
    let packet: PacketObservation =
        serde_json::from_value(event.get("fields").ok_or("missing circuit fields")?.clone())?;
    if packet.cid.len() != 64
        || !packet.cid.bytes().all(|byte| byte.is_ascii_hexdigit())
        || packet.path_json.len() > 512
    {
        return Err("invalid production circuit observation".into());
    }
    Ok(Some(packet))
}

pub(super) async fn original_path(signals: &Signals) -> Result<Vec<NodeId>> {
    let mut path = Vec::new();
    for role in Role::ALL {
        let Signal::Status {
            role: claimed,
            status,
        } = signals.wait(&format!("enrolled-{}", role.name())).await?
        else {
            return Err("missing enrolled identity for circuit path proof".into());
        };
        if claimed != role {
            return Err("circuit path role mismatch".into());
        }
        path.push(
            status
                .identity
                .ok_or("circuit path identity absent")?
                .node_id,
        );
    }
    Ok(path)
}

pub async fn sample(
    cli: &Cli,
    traffic: &Traffic,
    elapsed: Duration,
    stage: &str,
) -> Result<Sample> {
    let sample = Sample {
        observed_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .try_into()?,
        packet: traffic.evidence(elapsed)?,
    };
    tokio::fs::write(
        cli.evidence.join(format!("fallback-{stage}.json")),
        serde_json::to_vec_pretty(&sample)?,
    )
    .await?;
    Ok(sample)
}

pub async fn verify(
    cli: &Cli,
    signals: &Signals,
    before: Sample,
    after: Sample,
) -> Result<Evidence> {
    let path = cli.signals.join("fallback-blocked-time.json");
    if tokio::fs::metadata(&path).await?.len() > 1024 {
        return Err("fallback transition timestamp exceeds bound".into());
    }
    let blocked: Blocked = serde_json::from_slice(&tokio::fs::read(path).await?)?;
    if blocked.blocked_at_ms
        < before
            .observed_at_ms
            .saturating_add(cli.fallback_delay().as_millis().try_into()?)
        || blocked.blocked_at_ms >= after.observed_at_ms
    {
        return Err("fallback transition was outside live persistent traffic".into());
    }
    let original_path = original_path(signals).await?;
    let mut packets = Vec::new();
    for event in actor::log_events(cli).await? {
        if let Some(packet) = packet_observation(&event)? {
            packets.push(packet);
        }
    }
    let mut circuits = Vec::new();
    let mut before_cids = BTreeMap::<String, usize>::new();
    let mut after_cids = BTreeMap::<String, usize>::new();
    for packet in packets.iter().filter(|packet| {
        packet.observed_at_ms >= before.observed_at_ms
            && packet.observed_at_ms <= after.observed_at_ms
    }) {
        let path: Vec<NodeId> = serde_json::from_str(&packet.path_json)?;
        if path != original_path {
            return Err("fallback changed the original complete circuit path".into());
        }
        let counts = if packet.observed_at_ms < blocked.blocked_at_ms {
            &mut before_cids
        } else {
            &mut after_cids
        };
        *counts.entry(packet.cid.to_ascii_lowercase()).or_default() += 1;
    }
    if before_cids.len() != 1 || after_cids.len() != 1 || before_cids.keys().ne(after_cids.keys()) {
        return Err("missing or replaced real TCP circuit across fallback".into());
    }
    let (cid, before_packets) = before_cids
        .into_iter()
        .next()
        .ok_or("missing original TCP circuit")?;
    let after_packets = after_cids[&cid];
    circuits.push(Circuit {
        cid,
        before_packets,
        after_packets,
    });
    let evidence = Evidence {
        passed: true,
        original_path,
        before_at_ms: before.observed_at_ms,
        blocked_at_ms: blocked.blocked_at_ms,
        after_at_ms: after.observed_at_ms,
        circuits,
    };
    tokio::fs::write(
        cli.evidence.join("fallback-circuits.json"),
        serde_json::to_vec_pretty(&evidence)?,
    )
    .await?;
    Ok(evidence)
}
