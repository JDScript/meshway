use meshway_ipc::{EndpointKind, LinkAcquisition, PingEvent, ProbeOutcome};
use std::io::{self, Write};

pub(crate) fn display(event: &PingEvent, json: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    write_event(&mut output, event, json)?;
    output.flush()
}

fn write_event(output: &mut impl Write, event: &PingEvent, json: bool) -> io::Result<()> {
    if json {
        serde_json::to_writer(&mut *output, event)?;
        return writeln!(output);
    }
    match event {
        PingEvent::Accepted { target, .. } => {
            write!(output, "PING {} ({})", target.hostname, target.node_id)?;
            if let Some(address) = target.addresses.ipv4 {
                write!(output, " {address}")?;
            }
            if let Some(address) = target.addresses.ipv6 {
                write!(output, " {address}")?;
            }
            writeln!(output, "  service=icmp:* timeout=5s interval>=1s")
        }
        PingEvent::Preparing { elapsed_ms } => {
            writeln!(output, "preparing: elapsed={elapsed_ms} ms")
        }
        PingEvent::Sample { sample, .. } => {
            let Some(probe) = &sample.probe else {
                return writeln!(output, "sample {}: no probe data", sample.measurement_id);
            };
            write!(output, "sample {}: ", sample.measurement_id)?;
            match probe.outcome {
                ProbeOutcome::Success => {
                    write!(output, "reply RTT=")?;
                    if let Some(duration) = probe.duration_us {
                        // Exact microseconds -> milliseconds, without lossy float casts.
                        write!(
                            output,
                            "{}.{:03} ms",
                            duration.get() / 1000,
                            duration.get() % 1000
                        )?;
                    } else {
                        write!(output, "unavailable")?;
                    }
                }
                ProbeOutcome::Timeout => write!(output, "timeout")?,
                ProbeOutcome::Failed => write!(output, "local-failed")?,
                ProbeOutcome::Pending => write!(output, "pending")?,
            }
            if probe.path.len() > 2 {
                write!(output, " via ")?;
                for (index, node) in probe.path[1..probe.path.len() - 1].iter().enumerate() {
                    if index > 0 {
                        write!(output, " -> ")?;
                    }
                    // Only target hostname is authorized in Accepted. Intermediate
                    // names are not part of the contract, so retain actual NodeIds.
                    write!(output, "{node}")?;
                }
            } else if probe.path.len() == 2 {
                if let Some(channels) = &probe.channels {
                    let channel = &channels.request;
                    write!(
                        output,
                        " direct remote={} transport={} acquisition=",
                        channel.remote_address,
                        channel.transport.as_str()
                    )?;
                    match &channel.acquisition {
                        LinkAcquisition::PublishedEndpoint => write!(output, "publishedEndpoint")?,
                        LinkAcquisition::LocalEndpoint => write!(output, "localEndpoint")?,
                        LinkAcquisition::StunEndpoint => write!(output, "stunEndpoint")?,
                        LinkAcquisition::AcceptedInbound => write!(output, "acceptedInbound")?,
                        LinkAcquisition::CoordinatedUdp {
                            endpoint_source, ..
                        } => {
                            write!(output, "coordinatedUdp endpointSource=")?;
                            match endpoint_source {
                                EndpointKind::Stun => write!(output, "STUN (mapped endpoint)")?,
                                EndpointKind::Configured => write!(output, "configured")?,
                                EndpointKind::Interface => write!(output, "interface")?,
                            }
                        }
                    }
                } else {
                    write!(output, " channel unavailable")?;
                }
            } else {
                write!(output, " path unavailable")?;
            }
            writeln!(output)
        }
        PingEvent::Completed {
            sent,
            success,
            timeout,
            failed,
        } => writeln!(
            output,
            "sent={sent} replied={success} timeout={timeout} local-failed={failed}"
        ),
        PingEvent::Failed { code } => writeln!(output, "ping failed: {code:?}"),
    }
}

#[cfg(test)]
mod tests;
