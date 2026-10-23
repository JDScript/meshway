//! `meshway status`: this Node first, then the peers of its installed netmap.
use super::format::{ago, left, millis, words};
use super::table::{Cell, Table};
use super::term::{Role, Term, pad};
use chrono::{DateTime, Utc};
use meshway_protocol::ipc::{DataPlane, IdleRoute, PeerState, Phase, Status, SyncState};
use std::io::{self, Write};

fn phase_role(phase: Phase) -> Role {
    match phase {
        Phase::Running | Phase::ControlReady => Role::Good,
        Phase::Blocked => Role::Bad,
        Phase::Down => Role::Dim,
        _ => Role::Warn,
    }
}

fn line(output: &mut impl Write, term: &Term, label: &str, value: String) -> io::Result<()> {
    writeln!(
        output,
        "  {} {value}",
        term.paint(Role::Dim, pad(label, 10))
    )
}

pub(crate) fn write(
    output: &mut impl Write,
    term: &Term,
    status: &Status,
    now: DateTime<Utc>,
) -> io::Result<()> {
    let name = status
        .node
        .as_ref()
        .map(|node| node.hostname.clone())
        .unwrap_or_else(|| {
            if status.identity.is_some() {
                "this node".into()
            } else {
                "not enrolled".into()
            }
        });
    let mut title = term.paint(Role::Bold, &name);
    if let Some(node) = &status.node {
        for address in [
            node.addresses.ipv4.map(|a| a.to_string()),
            node.addresses.ipv6.map(|a| a.to_string()),
        ]
        .into_iter()
        .flatten()
        {
            title.push_str("  ");
            title.push_str(&term.paint(Role::Accent, address));
        }
    }
    title.push_str("  ");
    title.push_str(&term.paint(phase_role(status.phase), words(status.phase)));
    if !status.desired_up && status.phase != Phase::Down {
        title.push_str(&term.paint(Role::Dim, " (desired down)"));
    }
    writeln!(output, "{title}")?;

    line(output, term, "Server", status.server.clone())?;
    let mut sync = match status.sync {
        SyncState::Connected => term.paint(Role::Good, "connected"),
        SyncState::Reconnecting => term.paint(Role::Warn, "reconnecting"),
        SyncState::Disconnected => term.paint(Role::Dim, "disconnected"),
    };
    if let Some(error) = status.sync_error {
        sync.push_str(&format!(" ({})", words(error)));
    }
    if let Some(at) = status.last_sync_at {
        sync.push_str(&term.paint(Role::Dim, format!(" · last sync {}", ago(at, now))));
    }
    line(output, term, "Sync", sync)?;
    let netmap = match &status.netmap {
        Some(netmap) => {
            let mut text = format!(
                "revision {} · serial {} · lease {}",
                netmap.netmap.revision,
                netmap.netmap.serial.get(),
                left(netmap.expires_at, now)
            );
            if status.applied != Some(netmap.netmap) {
                text.push_str(&term.paint(Role::Warn, " · not applied"));
            }
            text
        }
        None => match &status.saved {
            Some(saved) => term.paint(
                Role::Warn,
                format!(
                    "no live lease (saved revision {} · serial {})",
                    saved.revision,
                    saved.serial.get()
                ),
            ),
            None => term.paint(Role::Dim, "none"),
        },
    };
    line(output, term, "Netmap", netmap)?;
    if let Some(capture) = status.netmap.as_ref().and_then(|n| n.capture.as_ref()) {
        let text = match capture.unavailable {
            None if capture.managed_ipv6 == 0 => format!(
                "{} IPv4 · {} IPv6 sink ranges · {} routes",
                capture.managed_ipv4, capture.sink_ipv6, capture.routes
            ),
            None => format!(
                "{} IPv4 · {} IPv6 · {} IPv6 sink ranges · {} routes",
                capture.managed_ipv4, capture.managed_ipv6, capture.sink_ipv6, capture.routes
            ),
            Some(reason) => term.paint(
                Role::Warn,
                format!("unavailable ({}) · full capture", words(reason)),
            ),
        };
        line(output, term, "Egress", text)?;
    }
    if let Some(routing) = status.netmap.as_ref().and_then(|n| n.routing.as_ref()) {
        let mut text = routing.applied_profile.clone();
        if routing.requested_unavailable.is_some() {
            text.push_str(&term.paint(Role::Warn, " · requested profile not applied"));
        }
        text.push_str(&term.paint(Role::Dim, " (meshway exit)"));
        line(output, term, "Profile", text)?;
    }
    let plane = term.paint(
        match status.data_plane {
            DataPlane::Ready => Role::Good,
            DataPlane::Failed => Role::Bad,
            DataPlane::ControlOnly | DataPlane::Stopped => Role::Dim,
            _ => Role::Warn,
        },
        words(status.data_plane),
    );
    line(
        output,
        term,
        "Data plane",
        format!(
            "{plane} · mode {}{}",
            match status.network_mode {
                meshway_protocol::ipc::NetworkMode::Tun => "tun",
                meshway_protocol::ipc::NetworkMode::ControlOnly => "control-only",
            },
            term.paint(
                Role::Dim,
                format!(" · changed {}", ago(status.observed_at, now))
            )
        ),
    )?;
    line(output, term, "Exit", exit(term, status.exit.as_ref()))?;
    if let Some(domains) = &status.local_dns_domains {
        let shown: Vec<_> = domains
            .iter()
            .map(|local| match &local.link_name {
                Some(name) => format!("{} ({name})", local.domain),
                None if local.link_index == 0 => format!("{} (global)", local.domain),
                None => format!("{} (link {})", local.domain, local.link_index),
            })
            .collect();
        line(
            output,
            term,
            "Local DNS",
            term.paint(
                Role::Dim,
                format!(
                    "{} · resolved by the local network, not the DNS fallback",
                    shown.join(", ")
                ),
            ),
        )?;
    }
    if let Some(identity) = &status.identity {
        line(
            output,
            term,
            "Identity",
            term.paint(
                Role::Dim,
                format!(
                    "node {} · network {} · key generation {}",
                    identity.node_id,
                    identity.network_id,
                    identity.keys.control.generation.get()
                ),
            ),
        )?;
    }
    if let Some(change) = &status.change_pending {
        let owner = match change.owner {
            meshway_protocol::ipc::KernelChangeOwner::EgressCapture => "egress capture",
            meshway_protocol::ipc::KernelChangeOwner::Exit => "exit role",
        };
        line(
            output,
            term,
            "Kernel",
            term.paint(
                Role::Warn,
                format!(
                    "{owner} {} pending ({}; attempt {}, retry in {})",
                    change.operation,
                    change.reason,
                    change.attempts,
                    super::format::span((change.retry_at - now).num_seconds().max(0))
                ),
            ),
        )?;
    }
    if let Some(relay) = &status.control_relay {
        let text = match relay.state {
            meshway_protocol::ipc::ControlRelayState::Retrying => format!(
                "retrying ({}; attempt {}, retry in {})",
                relay.reason,
                relay.attempts,
                super::format::span(
                    relay
                        .retry_at
                        .map_or(0, |at| (at - now).num_seconds().max(0))
                )
            ),
            meshway_protocol::ipc::ControlRelayState::AtCapacity => format!(
                "at capacity ({} limit; {} refused, current connections kept)",
                relay.reason, relay.attempts
            ),
        };
        line(output, term, "Relay", term.paint(Role::Warn, text))?;
    }
    if let Some(problem) = status.last_error {
        let mut text = term.paint(Role::Bad, words(problem));
        if let Some(at) = status.retry_at {
            text.push_str(&format!(
                " · retry in {}",
                super::format::span((at - now).num_seconds())
            ));
        }
        line(output, term, "Problem", text)?;
        if let Some(detail) = &status.last_error_detail {
            line(output, term, "", term.paint(Role::Bad, detail))?;
        }
    }
    if let Some(failure) = &status.network_error {
        line(
            output,
            term,
            "Network",
            term.paint(Role::Bad, words(failure.problem)),
        )?;
        if let Some(detail) = &failure.detail {
            line(output, term, "", term.paint(Role::Bad, detail))?;
        }
    }
    if let Some(url) = &status.enrollment_url {
        writeln!(output, "\nApprove this device in your browser:\n{url}")?;
    } else if status.phase == Phase::AwaitingApproval {
        writeln!(
            output,
            "\nAwaiting approval. The Server has not configured a Console origin; ask its operator to configure --console-origin (MESHWAY_CONSOLE_ORIGIN)."
        )?;
    }
    if status.node.is_some() {
        writeln!(output)?;
        if status.peers.is_empty() {
            writeln!(
                output,
                "{}",
                term.paint(Role::Dim, "No peers in the installed netmap.")
            )?;
        } else {
            peers(output, term, status)?;
        }
        if !status.sockets.is_empty() {
            writeln!(output)?;
            sockets(output, term, status)?;
        }
    }
    Ok(())
}

fn size(bytes: u32) -> String {
    let bytes = f64::from(bytes);
    if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", bytes / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB", bytes / 1024.0)
    }
}

/// Granted buffers as the kernel reports them; one that does not hold the
/// request ([`meshway_transport::raw::satisfies`]: Linux reports twice the
/// grant) means the kernel limited it (`net.core.rmem_max`／`wmem_max`). A
/// listener whose bind is being retried shows why it is unavailable.
fn sockets(output: &mut impl Write, term: &Term, status: &Status) -> io::Result<()> {
    let mut table = Table::new(vec!["SOCKET", "BOUND", "RECEIVE BUFFER", "SEND BUFFER"]);
    let cell = |granted: Option<u32>, requested: Option<u32>| match (granted, requested) {
        (Some(granted), Some(requested))
            if !meshway_transport::raw::satisfies(granted as usize, requested as usize) =>
        {
            Cell::styled(
                format!("{} (requested {}, limited)", size(granted), size(requested)),
                Role::Warn,
            )
        }
        (Some(granted), _) => Cell::new(size(granted)),
        (None, _) => Cell::new(""),
    };
    for socket in &status.sockets {
        let bound = match (&socket.bound_address, &socket.unavailable) {
            (_, Some(_)) => Cell::styled("unavailable", Role::Bad),
            (Some(address), None) => Cell::new(address.to_string()),
            (None, None) => Cell::new(""),
        };
        table.row(vec![
            Cell::new(super::netcheck::resource(&socket.resource, &status.peers)),
            bound,
            cell(socket.receive_buffer, socket.receive_requested),
            cell(socket.send_buffer, socket.send_requested),
        ]);
    }
    table.render(output, term)
}

fn exit(term: &Term, exit: Option<&meshway_protocol::ipc::ExitState>) -> String {
    let Some(exit) = exit else {
        return term.paint(Role::Dim, "none");
    };
    use meshway_model::exit::{ExitFamily, ExitStatus};
    let status = |status: &ExitStatus| match status {
        ExitStatus::NotRequested => "not requested".into(),
        ExitStatus::Probing => term.paint(Role::Warn, "probing"),
        ExitStatus::Ready => term.paint(Role::Good, "ready"),
        ExitStatus::Unavailable { reason } => {
            term.paint(Role::Bad, format!("unavailable ({})", words(reason)))
        }
    };
    let families = exit
        .capability
        .as_ref()
        .map(|capability| capability.families.as_slice())
        .unwrap_or_default();
    // One family reads as before families; each of two is named.
    let mut text = match families {
        [] => "not requested".into(),
        [only] => status(&only.status),
        families => families
            .iter()
            .map(|entry| {
                let family = match entry.family {
                    ExitFamily::Ipv4 => "IPv4",
                    ExitFamily::Ipv6 => "IPv6",
                };
                format!("{family} {}", status(&entry.status))
            })
            .collect::<Vec<_>>()
            .join(", "),
    };
    if exit.installed_role.is_some() {
        text.push_str(" · role installed");
    }
    if let Some(flows) = exit.active_flows {
        text.push_str(&format!(" · {flows} flows"));
    }
    if exit.dns_unavailable.is_some() {
        text.push_str(&term.paint(Role::Warn, " · exit DNS unavailable"));
    }
    if let Some(cleanup) = &exit.cleanup_pending {
        text.push_str(&term.paint(
            Role::Warn,
            format!(
                " · cleanup pending ({}; attempt {}, retrying)",
                cleanup.reason, cleanup.attempts
            ),
        ));
    }
    text
}

fn peers(output: &mut impl Write, term: &Term, status: &Status) -> io::Result<()> {
    let name = |id: &meshway_model::NodeId| {
        status
            .peers
            .iter()
            .find(|peer| peer.node_id == *id)
            .map(|peer| peer.hostname.clone())
            .unwrap_or_else(|| id.to_string())
    };
    let mut table = Table::new(vec![
        "PEER",
        "MESH IPV4",
        "STATE",
        "TRANSPORT",
        "ENDPOINT",
        "MTU",
        "RTT",
    ]);
    for peer in &status.peers {
        let (state, role) = match peer.state {
            PeerState::Direct => ("direct", Role::Good),
            PeerState::Relay => ("relay", Role::Accent),
            PeerState::RelayIdle => ("relay (idle)", Role::Dim),
            PeerState::Idle => ("idle", Role::Dim),
            PeerState::Unreachable => ("unreachable", Role::Warn),
            PeerState::Offline => ("offline", Role::Bad),
        };
        let via = |via: &[meshway_model::NodeId]| {
            format!(
                "via {}",
                via.iter().map(name).collect::<Vec<_>>().join(" → ")
            )
        };
        // Without a channel or circuit: the path a new circuit would take.
        let endpoint = match (&peer.state, &peer.idle_path) {
            (PeerState::Direct, _) => peer
                .endpoint
                .map(|endpoint| endpoint.to_string())
                .unwrap_or_default(),
            (PeerState::Relay, _) => via(&peer.via),
            (_, Some(idle)) => match idle.route {
                IdleRoute::Direct => "direct (idle)".to_owned(),
                IdleRoute::Relay => format!("relay (idle) {}", via(&idle.via)),
                IdleRoute::None => "no path".to_owned(),
            },
            (PeerState::RelayIdle, None) => via(&peer.via),
            _ => String::new(),
        };
        table.row(vec![
            Cell::new(&peer.hostname),
            Cell::styled(
                peer.addresses
                    .ipv4
                    .map(|address| address.to_string())
                    .unwrap_or_default(),
                Role::Accent,
            ),
            Cell::styled(state, role),
            Cell::new(peer.transport.map(|t| t.as_str()).unwrap_or_default()),
            Cell::new(endpoint),
            // Outer path MTU / largest inner packet of a direct UDP channel.
            Cell::new(
                peer.mtu
                    .map(|mtu| format!("{}/{}", mtu.path, mtu.inner))
                    .unwrap_or_default(),
            ),
            Cell::new(
                peer.rtt
                    .as_ref()
                    .map(|rtt| millis(rtt.duration_us.get()))
                    .unwrap_or_default(),
            ),
        ]);
    }
    table.render(output, term)
}

#[cfg(test)]
mod tests;
