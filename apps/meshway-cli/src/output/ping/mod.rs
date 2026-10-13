//! `meshway ping`: the local decision, one line per probe and a summary;
//! `--json` keeps the exact typed event stream.
mod external;
use super::format::{millis, words};
use super::term::{Role, Term};
use meshway_model::NodeId;
use meshway_protocol::NodeObservationTargetView;
use meshway_protocol::ipc::{EndpointKind, LinkAcquisition, PeerStatus, PingEvent, ProbeOutcome};
use std::collections::BTreeMap;
use std::io::{self, Write};

pub(crate) struct Ping {
    verbose: bool,
    target: Option<NodeObservationTargetView>,
    /// The decision of a target outside the Mesh.
    decision: Option<meshway_protocol::ipc::PingDecision>,
    /// Successful round trips in microseconds, for the summary.
    rtts: Vec<u64>,
    /// Echo results answered by an ICMP error, for the summary.
    unreachable: u32,
    /// Hostnames of this Node and its visible peers, for paths.
    names: Option<BTreeMap<NodeId, String>>,
}

impl Ping {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
            target: None,
            decision: None,
            rtts: Vec::new(),
            unreachable: 0,
            names: None,
        }
    }

    /// The event names Nodes this view cannot name yet.
    pub fn wants_names(&self, event: &PingEvent) -> bool {
        self.names.is_none()
            && match event {
                PingEvent::Sample { sample, .. } => sample
                    .probe
                    .as_ref()
                    .is_some_and(|probe| probe.path.len() > 2),
                PingEvent::Decision { decision } => {
                    !decision.path.is_empty()
                        || matches!(
                            decision.resolution.as_ref().map(|r| &r.resolver),
                            Some(meshway_protocol::ipc::PingResolver::Exit { .. })
                        )
                }
                _ => false,
            }
    }

    /// Names of this Node (`own`) and its visible peers.
    pub fn names(&mut self, peers: &[PeerStatus], own: Option<(NodeId, String)>) {
        self.names = Some(
            peers
                .iter()
                .map(|peer| (peer.node_id, peer.hostname.clone()))
                .chain(own)
                .collect(),
        );
    }

    fn name(&self, id: &NodeId) -> String {
        self.names
            .as_ref()
            .and_then(|names| names.get(id).cloned())
            .unwrap_or_else(|| id.to_string())
    }

    fn peer(&self) -> String {
        match &self.target {
            Some(target) => {
                let address = target
                    .addresses
                    .ipv4
                    .map(|a| a.to_string())
                    .or(target.addresses.ipv6.map(|a| a.to_string()));
                match address {
                    Some(address) => format!("{} ({address})", target.hostname),
                    None => target.hostname.clone(),
                }
            }
            None => "peer".into(),
        }
    }

    pub fn write(
        &mut self,
        output: &mut impl Write,
        term: &Term,
        event: &PingEvent,
    ) -> io::Result<()> {
        match event {
            PingEvent::Decision { decision } => self.decision(output, term, decision),
            PingEvent::Echo { echo } => self.echo(output, term, echo),
            PingEvent::Accepted { target, .. } => {
                self.target = Some(target.clone());
                if self.verbose {
                    writeln!(
                        output,
                        "{}",
                        term.paint(
                            Role::Dim,
                            format!(
                                "PING {} node {} · icmp:* · 5 s timeout · ≥1 s interval",
                                self.peer(),
                                target.node_id
                            )
                        )
                    )?;
                }
                Ok(())
            }
            PingEvent::Preparing { elapsed_ms } => {
                if self.verbose {
                    writeln!(
                        output,
                        "{}",
                        term.paint(Role::Dim, format!("preparing ({} ms)", elapsed_ms.get()))
                    )?;
                }
                Ok(())
            }
            PingEvent::Sample { sample, .. } => {
                let Some(probe) = &sample.probe else {
                    return writeln!(
                        output,
                        "{}",
                        term.paint(Role::Warn, "sample without probe data")
                    );
                };
                let peer = self.peer();
                match probe.outcome {
                    ProbeOutcome::Success => {}
                    ProbeOutcome::Timeout => {
                        return writeln!(
                            output,
                            "{}",
                            term.paint(Role::Warn, format!("no reply from {peer} within 5 s"))
                        );
                    }
                    ProbeOutcome::Failed => {
                        return writeln!(
                            output,
                            "{}",
                            term.paint(Role::Bad, format!("probe to {peer} failed locally"))
                        );
                    }
                    ProbeOutcome::Pending => {
                        return writeln!(
                            output,
                            "{}",
                            term.paint(Role::Dim, format!("probe to {peer} pending"))
                        );
                    }
                }
                let channel = probe.channels.as_ref().map(|channels| &channels.request);
                let carrier = channel.map(|channel| {
                    format!(
                        "{} {}",
                        term.paint(Role::Accent, channel.remote_address),
                        channel.transport.as_str()
                    )
                });
                let route = if probe.path.len() > 2 {
                    let relays: Vec<String> = probe.path[1..probe.path.len() - 1]
                        .iter()
                        .map(|id| self.name(id))
                        .collect();
                    match &carrier {
                        Some(carrier) => format!("via relay {} ({carrier})", relays.join(" → ")),
                        None => format!("via relay {}", relays.join(" → ")),
                    }
                } else {
                    match &carrier {
                        Some(carrier) => format!("via {carrier}"),
                        None => "via an unreported channel".into(),
                    }
                };
                let rtt = match probe.duration_us {
                    Some(duration) => {
                        self.rtts.push(duration.get());
                        format!("in {}", term.paint(Role::Bold, millis(duration.get())))
                    }
                    None => "with no measured round trip".into(),
                };
                write!(
                    output,
                    "{} from {peer} {route} {rtt}",
                    term.paint(Role::Good, "pong")
                )?;
                if self.verbose
                    && let Some(channel) = channel
                {
                    write!(
                        output,
                        "{}",
                        term.paint(
                            Role::Dim,
                            format!(" [{}]", acquisition(&channel.acquisition))
                        )
                    )?;
                }
                writeln!(output)
            }
            PingEvent::Completed { .. } if self.decision.is_some() => {
                self.external_summary(output, term, event)
            }
            PingEvent::Completed {
                sent,
                success,
                timeout,
                failed,
            } => {
                let loss = if *sent == 0 {
                    0
                } else {
                    u64::from(*timeout) * 100 / u64::from(*sent)
                };
                let mut text = format!(
                    "{}: {sent} sent, {success} replied, {loss}% loss",
                    self.target
                        .as_ref()
                        .map(|target| target.hostname.clone())
                        .unwrap_or_else(|| "ping".into())
                );
                if *failed > 0 {
                    text.push_str(&format!(", {failed} failed locally"));
                }
                text.push_str(&self.rtt_summary());
                writeln!(output, "{}", term.paint(Role::Bold, text))
            }
            PingEvent::Failed { code } => writeln!(
                output,
                "{}",
                term.paint(Role::Bad, format!("ping failed: {}", words(code)))
            ),
        }
    }
}

impl Ping {
    fn rtt_summary(&self) -> String {
        match (self.rtts.iter().min(), self.rtts.iter().max()) {
            (Some(min), Some(max)) => {
                let average = self.rtts.iter().sum::<u64>() / self.rtts.len() as u64;
                format!(
                    " · rtt min/avg/max {}/{}/{}",
                    millis(*min).trim_end_matches(" ms"),
                    millis(average).trim_end_matches(" ms"),
                    millis(*max)
                )
            }
            _ => String::new(),
        }
    }
}

fn acquisition(acquisition: &LinkAcquisition) -> String {
    match acquisition {
        LinkAcquisition::PublishedEndpoint => "published endpoint".into(),
        LinkAcquisition::LocalEndpoint => "local endpoint".into(),
        LinkAcquisition::StunEndpoint => "STUN endpoint".into(),
        LinkAcquisition::AcceptedInbound => "accepted inbound".into(),
        LinkAcquisition::CoordinatedUdp {
            endpoint_source, ..
        } => format!(
            "coordinated UDP, {} endpoint",
            match endpoint_source {
                EndpointKind::Stun => "STUN-mapped",
                EndpointKind::Configured => "configured",
                EndpointKind::Interface => "interface",
            }
        ),
    }
}

/// NDJSON: the exact typed event.
pub(crate) fn json(output: &mut impl Write, event: &PingEvent) -> io::Result<()> {
    serde_json::to_writer(&mut *output, event)?;
    writeln!(output)
}

#[cfg(test)]
mod tests;
