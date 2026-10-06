//! Evidence from the production CLI's JSON-lines stream only. This module never
//! creates probes, accesses Node credentials, or infers a channel from State.
use crate::Result;
use meshway_ipc::{PingError, PingEvent, Status};
use meshway_model::{
    ConfigurationRef, InstanceId, NetworkId, NodeId, ObservationId, ObservationSubscriptionId,
    Version,
    observations::ObservationSample,
    state::{ProbeKind, ProbeOutcome},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const RUN_LIMIT: Duration = Duration::from_secs(90);
/// Includes the two independently bounded real CLI status calls.
pub const STAGE_LIMIT: Duration = Duration::from_secs(110);
pub const INITIAL_LIMIT: Duration = Duration::from_secs(600);
pub(super) const OUTPUT_LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub node: NodeId,
    pub network: NetworkId,
    pub process_id: u32,
    pub operator_revision: Version,
    pub instance: Option<InstanceId>,
    pub applied: Option<ConfigurationRef>,
    pub desired_up: bool,
}
impl Identity {
    pub(super) fn from_status(status: Status) -> Result<Self> {
        let identity = status.identity.ok_or("ping actor identity missing")?;
        Ok(Self {
            node: identity.node_id,
            network: identity.network_id,
            process_id: status.process_id,
            operator_revision: status.revision,
            instance: status.instance.map(|lease| lease.instance_id),
            applied: status.applied,
            desired_up: status.desired_up,
        })
    }
    fn same_operator(&self, other: &Self) -> bool {
        self.node == other.node
            && self.network == other.network
            && self.process_id == other.process_id
            && self.operator_revision == other.operator_revision
            && self.desired_up
            && other.desired_up
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Hostname,
    Ipv4,
    Ipv6,
    Direct,
    Shared,
    UdpTcpOnly,
    Revoked,
    Restored,
    Interrupted,
    Outage,
    Expired,
    Recovered,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Run {
    pub stage: Stage,
    pub command: Vec<String>,
    pub selector: String,
    pub child_pid: u32,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    /// Process lifetime, never represented as a probe RTT.
    pub elapsed_ms: u64,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub sent_signal: Option<i32>,
    pub before: Identity,
    pub after: Identity,
    pub events: Vec<PingEvent>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PingEvidence {
    pub runs: Vec<Run>,
    pub business_continuity: Option<BusinessContinuity>,
    pub diagnostic_only_business_demands: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BusinessContinuity {
    pub sockets_before: [std::net::SocketAddr; 4],
    pub sockets_after: [std::net::SocketAddr; 4],
    pub counts_before: crate::application::Counts,
    pub counts_after: crate::application::Counts,
}

impl PingEvidence {
    pub fn validate(&self, node: NodeId, recovery: &crate::recovery::Evidence) -> Result<()> {
        if self.diagnostic_only_business_demands != Some(0) {
            return Err("initial CLI diagnostics were preceded by business demand".into());
        }
        let stages = [
            Stage::Hostname,
            Stage::Ipv4,
            Stage::Ipv6,
            Stage::Direct,
            Stage::Shared,
            Stage::UdpTcpOnly,
            Stage::Revoked,
            Stage::Restored,
            Stage::Interrupted,
            Stage::Outage,
            Stage::Expired,
            Stage::Recovered,
        ];
        if self.runs.len() != stages.len()
            || !self
                .runs
                .iter()
                .zip(stages)
                .all(|(run, stage)| run.stage == stage)
        {
            return Err("missing, duplicate or reordered CLI ping stages".into());
        }
        let path = &self.runs[0]
            .samples()
            .next()
            .and_then(|sample| sample.probe.as_ref())
            .ok_or("initial CLI path missing")?
            .path;
        if path.len() != 3
            || path[0] != node
            || path.iter().collect::<std::collections::BTreeSet<_>>().len() != 3
        {
            return Err("invalid full CLI diagnostic path".into());
        }
        let mut measurements = std::collections::BTreeSet::new();
        for run in &self.runs {
            run.validate(if run.stage == Stage::Direct {
                &path[..2]
            } else {
                path
            })?;
            for sample in run.samples() {
                if !measurements.insert(sample.measurement_id) {
                    return Err("new CLI replayed an old measurement".into());
                }
            }
        }
        let old = &self.runs[9];
        let expired = &self.runs[10];
        let new = &self.runs[11];
        if old.before.instance.is_none()
            || expired.before.instance.is_some()
            || expired.before.applied.is_some()
            || new.before.instance.is_none()
            || new.before.instance == old.before.instance
            || !old.before.same_operator(&new.after)
        {
            return Err(
                "ping automatic recovery reused old instance/authority or changed intent".into(),
            );
        }
        let expiry = i64::try_from(recovery.expired.instance_expires_at_ms)?;
        for sample in old.samples() {
            let probe = sample.probe.as_ref().ok_or("old ping sample lacks probe")?;
            if (probe.outcome == ProbeOutcome::Success
                && probe
                    .completed_at
                    .is_none_or(|at| at.timestamp_millis() > expiry))
                || (sample
                    .probe_counts
                    .as_ref()
                    .is_some_and(|counts| counts.sent > 0)
                    && probe.started_at.timestamp_millis() > expiry)
            {
                return Err(
                    "old ping continued sending or completing after instance expiry".into(),
                );
            }
        }
        if new.started_at_ms < recovery.reopened_at_ms
            || new.samples().any(|sample| {
                sample.window_start.timestamp_millis()
                    < i64::try_from(recovery.reopened_at_ms).unwrap_or(i64::MAX)
            })
        {
            return Err("recovered CLI replayed a pre-recovery sample".into());
        }
        let continuity = self
            .business_continuity
            .as_ref()
            .ok_or("ping business continuity counters missing")?;
        if continuity.sockets_before != continuity.sockets_after
            || !continuity
                .counts_after
                .since(continuity.counts_before)?
                .all_protocols()
        {
            return Err("CLI completion/SIGINT stopped original business sockets".into());
        }
        Ok(())
    }
}

/// Same DTO/parser for live stdout, retained files, and controller rendezvous.
pub fn decode_stdout(bytes: &[u8]) -> Result<Vec<PingEvent>> {
    if bytes.len() as u64 > OUTPUT_LIMIT || (!bytes.is_empty() && !bytes.ends_with(b"\n")) {
        return Err("ping stdout exceeded bound or ended mid-event".into());
    }
    std::str::from_utf8(bytes)?
        .lines()
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
}

impl Run {
    pub fn accepted(&self) -> Result<(ObservationSubscriptionId, ObservationId)> {
        let mut accepted = self.events.iter().filter_map(|event| match event {
            PingEvent::Accepted {
                subscription_id,
                observation_id,
                ..
            } => Some((*subscription_id, *observation_id)),
            _ => None,
        });
        let value = accepted.next().ok_or("CLI ping was not accepted")?;
        if accepted.next().is_some() {
            return Err("duplicate ping acceptance".into());
        }
        Ok(value)
    }
    pub fn samples(&self) -> impl Iterator<Item = &ObservationSample> {
        self.events.iter().filter_map(|event| match event {
            PingEvent::Sample { sample, .. } => Some(sample),
            _ => None,
        })
    }
    pub fn validate(&self, path: &[NodeId]) -> Result<()> {
        let selector_matches = match self.stage {
            Stage::Ipv4 => self.selector.parse::<std::net::Ipv4Addr>().is_ok(),
            Stage::Ipv6 => self.selector.parse::<std::net::Ipv6Addr>().is_ok(),
            Stage::Direct => self.selector == "r",
            _ => self.selector == "b",
        };
        let count = if matches!(self.stage, Stage::Interrupted | Stage::Outage) {
            "1000"
        } else {
            "2"
        };
        if !selector_matches
            || self.command.len() != 8
            || self.command[1] != "--socket"
            || self.command[3] != "ping"
            || self.command[4] != self.selector
            || self.command[5] != "--count"
            || self.command[6] != count
            || self.command[7] != "--json"
        {
            return Err("ping evidence command/selector does not match its stage".into());
        }
        if !self.before.same_operator(&self.after)
            || self.child_pid == self.before.process_id
            || self.finished_at_ms < self.started_at_ms
            || path.first() != Some(&self.before.node)
        {
            return Err("CLI ping altered operator intent/identity or invalid lifecycle".into());
        }
        let denied = matches!(
            self.stage,
            Stage::UdpTcpOnly | Stage::Revoked | Stage::Expired
        );
        if denied {
            let expected = if self.stage == Stage::Expired {
                PingError::NotRunning
            } else {
                PingError::PermissionDenied
            };
            if self.exit_code == Some(0)
                || self.events.iter().any(|event| {
                    matches!(
                        event,
                        PingEvent::Sample { .. }
                            | PingEvent::Accepted { .. }
                            | PingEvent::Completed { .. }
                    )
                })
                || !matches!(self.events.last(), Some(PingEvent::Failed { code }) if *code == expected)
            {
                return Err(
                    "denied ping emitted acceptance/sample/sent or wrong typed error".into(),
                );
            }
            return Ok(());
        }
        let (_, id) = self.accepted()?;
        let mut seen = std::collections::BTreeSet::new();
        let mut accepted = false;
        let mut terminal = false;
        let mut count = 0;
        for event in &self.events {
            if terminal {
                return Err("ping emitted events after terminal".into());
            }
            match event {
                PingEvent::Accepted { target, .. } => {
                    accepted = true;
                    if Some(&target.node_id) != path.last()
                        || target.hostname != if path.len() == 2 { "r" } else { "b" }
                    {
                        return Err("ping resolved wrong Node/hostname".into());
                    }
                    if let Ok(ip) = self.selector.parse::<std::net::IpAddr>() {
                        let matches = match ip {
                            std::net::IpAddr::V4(ip) => target.addresses.ipv4 == Some(ip),
                            std::net::IpAddr::V6(ip) => target.addresses.ipv6 == Some(ip),
                        };
                        if !matches {
                            return Err("ping Mesh IP selector resolved wrong addresses".into());
                        }
                    }
                }
                PingEvent::Sample {
                    observation_id,
                    sample,
                } => {
                    if !accepted || *observation_id != id || !seen.insert(sample.measurement_id) {
                        return Err("ping reused a measurement or mismatched task".into());
                    }
                    if self.stage == Stage::Outage
                        && sample
                            .probe
                            .as_ref()
                            .is_some_and(|probe| probe.outcome != ProbeOutcome::Success)
                    {
                        let probe = sample.probe.as_ref().ok_or("outage sample lacks probe")?;
                        if probe.path != path || probe.duration_us.is_some() {
                            return Err("outage failure fabricated RTT/path".into());
                        }
                    } else {
                        validate_sample(sample, path)?;
                    }
                    count += 1;
                }
                PingEvent::Completed {
                    sent,
                    success,
                    timeout,
                    failed,
                } => {
                    if *sent != count
                        || *success != count
                        || *timeout != 0
                        || *failed != 0
                        || count != 2
                    {
                        return Err("ping terminal counters do not match real samples".into());
                    }
                    terminal = true;
                }
                PingEvent::Failed { .. } => terminal = true,
                PingEvent::Preparing { .. } => {}
            }
        }
        match self.stage {
            Stage::Interrupted
                if self.sent_signal == Some(2) && self.exit_code == Some(130) && count > 0 => {}
            Stage::Outage
                if self.exit_code != Some(0)
                    && count > 0
                    && self.samples().any(|sample| {
                        sample
                            .probe
                            .as_ref()
                            .is_some_and(|probe| probe.outcome == ProbeOutcome::Success)
                    })
                    && matches!(self.events.last(), Some(PingEvent::Failed { .. })) => {}
            Stage::Interrupted | Stage::Outage => {
                return Err("live ping did not end at its required lifecycle boundary".into());
            }
            _ if self.exit_code == Some(0)
                && matches!(self.events.last(), Some(PingEvent::Completed { .. })) => {}
            _ => return Err("ping has no successful real completion".into()),
        }
        Ok(())
    }
}

pub fn validate_sample(sample: &ObservationSample, path: &[NodeId]) -> Result<()> {
    if path.len() < 2 {
        return Err("diagnostic sample has no source/target path".into());
    }
    let probe = sample
        .probe
        .as_ref()
        .ok_or("CLI sample lacks measured probe")?;
    let channels = probe
        .channels
        .as_ref()
        .ok_or("CLI sample lacks actual physical channels")?;
    let reply = channels
        .reply
        .as_ref()
        .ok_or("CLI sample lacks authenticated reply channel")?;
    if sample.partial
        || probe.measurement_id != sample.measurement_id
        || probe.kind != ProbeKind::Path
        || probe.path != path
        || probe.outcome != ProbeOutcome::Success
        || probe.duration_us.is_none()
        || probe.completed_at.is_none()
        || probe.started_at > probe.completed_at.ok_or("missing completion")?
        || channels.request.peer_node_id != path[1]
        || reply.peer_node_id != path[1]
        || probe
            .transport
            .as_deref()
            .is_some_and(|transport| transport != channels.request.transport.as_str())
        || probe
            .link_id
            .as_ref()
            .is_some_and(|id| *id != channels.request.link_id.to_string())
    {
        return Err(
            "ping is not a measured complete-path Noise reply with actual source channel".into(),
        );
    }
    let counts = sample
        .probe_counts
        .as_ref()
        .ok_or("ping sample lacks actual probe counters")?;
    if counts.sent != 1
        || counts.replied_within_deadline != 1
        || counts.timeout != 0
        || counts.pending != 0
    {
        return Err("ping sample probe counters disagree with success".into());
    }
    Ok(())
}

#[cfg(test)]
// Serialization input only; this deliberately has one sample, so it cannot
// pass the real two-reply run validator or stand in for Linux evidence.
pub(super) fn roundtrip_run(elapsed_ms: u64) -> Run {
    use meshway_model::{
        Sequence,
        observations::{ProbeChannel, ProbeChannels, ProbeWindow},
        state::ProbeObservation,
    };
    let a: NodeId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
    let r: NodeId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    let b: NodeId = "00000000-0000-4000-8000-000000000003".parse().unwrap();
    let id = "00000000-0000-4000-8000-000000000004";
    let identity = Identity {
        node: a,
        network: id.parse().unwrap(),
        process_id: 42,
        operator_revision: Version::MIN,
        instance: None,
        applied: None,
        desired_up: true,
    };
    let channel = ProbeChannel {
        link_id: [1; 32].into(),
        socket_id: [2; 32].into(),
        peer_node_id: r,
        transport: meshway_transport_spec::TransportProtocol::NativeUdp,
        local_address: "192.0.2.1:35000".parse().unwrap(),
        remote_address: "192.0.2.2:35000".parse().unwrap(),
        acquisition: meshway_ipc::LinkAcquisition::CoordinatedUdp {
            attempt_id: id.parse().unwrap(),
            endpoint_source: meshway_ipc::EndpointKind::Stun,
        },
    };
    let at = "2026-10-05T00:00:00Z".parse().unwrap();
    let sample = ObservationSample {
        measurement_id: id.parse().unwrap(),
        version: Version::MIN,
        window_start: at,
        window_end: at,
        partial: false,
        traffic: None,
        probe_counts: Some(ProbeWindow {
            sent: 1,
            replied_within_deadline: 1,
            timeout: 0,
            pending: 0,
        }),
        probe: Some(ProbeObservation {
            measurement_id: id.parse().unwrap(),
            kind: ProbeKind::Path,
            link_id: None,
            transport: None,
            channels: Some(ProbeChannels {
                request: channel.clone(),
                reply: Some(channel),
            }),
            path: vec![a, r, b],
            publication_id: id.parse().unwrap(),
            started_at: at,
            completed_at: Some(at),
            outcome: ProbeOutcome::Success,
            duration_us: Some(Sequence::MAX),
        }),
    };
    Run {
        stage: Stage::Hostname,
        command: vec![
            "meshway".into(),
            "ping".into(),
            "b".into(),
            "--count".into(),
            "2".into(),
            "--json".into(),
        ],
        selector: "b".into(),
        child_pid: 43,
        started_at_ms: 0,
        finished_at_ms: elapsed_ms,
        elapsed_ms,
        exit_code: Some(0),
        exit_signal: None,
        sent_signal: None,
        before: identity.clone(),
        after: identity,
        events: vec![
            PingEvent::Accepted {
                subscription_id: id.parse().unwrap(),
                observation_id: id.parse().unwrap(),
                target: meshway_protocol::NodeObservationTargetView {
                    node_id: b,
                    hostname: "b".into(),
                    addresses: meshway_configuration::NodeAddresses {
                        ipv4: Some("100.64.0.3".parse().unwrap()),
                        ipv6: Some("fc00::3".parse().unwrap()),
                    },
                },
            },
            PingEvent::Preparing {
                elapsed_ms: Sequence::MAX,
            },
            PingEvent::Sample {
                observation_id: id.parse().unwrap(),
                sample,
            },
            PingEvent::Completed {
                sent: 1,
                success: 1,
                timeout: 0,
                failed: 0,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::Signal;

    #[test]
    fn stdout_and_tagged_rendezvous_preserve_actual_channels_and_safe_integer_time() {
        for elapsed in [125_001, u64::MAX] {
            let run = roundtrip_run(elapsed);
            let mut stdout = Vec::new();
            for event in &run.events {
                serde_json::to_writer(&mut stdout, event).unwrap();
                stdout.push(b'\n');
            }
            let decoded = decode_stdout(&stdout).unwrap();
            assert_eq!(
                serde_json::to_vec(&decoded).unwrap(),
                serde_json::to_vec(&run.events).unwrap()
            );
            let signal = Signal::PingRun { run: Box::new(run) };
            let bytes = serde_json::to_vec(&signal).unwrap();
            let decoded = Signal::decode("ping-hostname", &bytes).unwrap();
            assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
        }
    }

    #[test]
    fn rejects_missing_reply_wrong_actual_peer_and_missing_rtt() {
        let run = roundtrip_run(1);
        let sample = run.samples().next().unwrap();
        let path = &sample.probe.as_ref().unwrap().path;
        validate_sample(sample, path).unwrap();
        let mut no_reply = sample.clone();
        no_reply
            .probe
            .as_mut()
            .unwrap()
            .channels
            .as_mut()
            .unwrap()
            .reply = None;
        assert!(validate_sample(&no_reply, path).is_err());
        let mut wrong_peer = sample.clone();
        wrong_peer
            .probe
            .as_mut()
            .unwrap()
            .channels
            .as_mut()
            .unwrap()
            .request
            .peer_node_id = path[2];
        assert!(validate_sample(&wrong_peer, path).is_err());
        let mut no_rtt = sample.clone();
        no_rtt.probe.as_mut().unwrap().duration_us = None;
        assert!(validate_sample(&no_rtt, path).is_err());
        assert!(decode_stdout(b"{\"unknown\":true}\n").is_err());
        assert!(decode_stdout(b"{\"unknown\":true}").is_err());
    }
}
