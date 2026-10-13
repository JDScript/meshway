//! `meshway netcheck`: what this Node's own sockets and Server client observed.
use super::format::{millis, words};
use super::table::{Cell, Table};
use super::term::{Role, Term, pad};
use meshway_model::state::ResourceRef;
use meshway_protocol::ipc::{EndpointMapping, NetcheckReport, PortMapping};
use std::io::{self, Write};

fn yes_no(term: &Term, value: bool) -> String {
    if value {
        term.paint(Role::Good, "yes")
    } else {
        term.paint(Role::Warn, "no")
    }
}

fn line(output: &mut impl Write, term: &Term, label: &str, value: String) -> io::Result<()> {
    writeln!(
        output,
        "  {} {value}",
        term.paint(Role::Dim, pad(label, 10))
    )
}

/// A socket's owning resource; `peers` names the far end of dial sockets,
/// whose keys carry its Node ID.
pub(crate) fn resource(
    resource: &ResourceRef,
    peers: &[meshway_protocol::ipc::PeerStatus],
) -> String {
    match resource {
        ResourceRef::Listener { key } => format!("listener {key}"),
        ResourceRef::Dial { key } => {
            let named = key.split_once('/').and_then(|(node, listener)| {
                peers
                    .iter()
                    .find(|peer| peer.node_id.to_string() == node)
                    .map(|peer| format!("dial {}/{listener}", peer.hostname))
            });
            named.unwrap_or_else(|| format!("dial {key}"))
        }
        ResourceRef::Coordination { key } => format!("coordination {key}"),
    }
}

/// `peers` names the far end of dial sockets, whose keys carry its Node ID.
pub(crate) fn write(
    output: &mut impl Write,
    term: &Term,
    report: &NetcheckReport,
    peers: &[meshway_protocol::ipc::PeerStatus],
) -> io::Result<()> {
    writeln!(output, "{}", term.paint(Role::Bold, "Network check"))?;
    let server = match (report.server.reachable, report.server.rtt_us) {
        (Some(true), Some(rtt)) => format!(
            "{} · {} in {}",
            report.server.origin,
            term.paint(Role::Good, "reachable"),
            millis(rtt.get())
        ),
        (Some(true), None) => format!(
            "{} · {}",
            report.server.origin,
            term.paint(Role::Good, "reachable")
        ),
        (Some(false), _) => format!(
            "{} · {}",
            report.server.origin,
            term.paint(Role::Bad, "unreachable")
        ),
        (None, _) => format!(
            "{} · {}",
            report.server.origin,
            term.paint(Role::Dim, "not probed")
        ),
    };
    line(output, term, "Server", server)?;
    if let Some(reason) = report.unavailable {
        line(
            output,
            term,
            "Network",
            term.paint(Role::Warn, format!("not checked ({})", words(reason))),
        )?;
        return Ok(());
    }
    line(output, term, "UDP", yes_no(term, report.udp))?;
    line(output, term, "IPv4", yes_no(term, report.ipv4))?;
    line(output, term, "IPv6", yes_no(term, report.ipv6))?;
    if report.observers.is_empty() {
        line(
            output,
            term,
            "STUN",
            term.paint(Role::Warn, "no observer answered"),
        )?;
    }
    for observer in &report.observers {
        let mut text = format!(
            "{} {}",
            observer.observer,
            observer
                .rtt_us
                .map(|rtt| format!("in {}", millis(rtt.get())))
                .unwrap_or_else(|| "(round trip ambiguous)".into())
        );
        // A DF probe answer proves the size; no answer measures nothing.
        if let Some(probe) = observer.size_probe {
            text.push_str(&if probe.answered {
                format!(
                    " · {} B DF {}",
                    probe.size,
                    term.paint(Role::Good, "answered")
                )
            } else {
                format!(
                    " · {} B DF {}",
                    probe.size,
                    term.paint(Role::Dim, "unanswered")
                )
            });
        }
        if let Some(mtu) = observer.path_mtu {
            text.push_str(&format!(" · route MTU {mtu}"));
        }
        line(output, term, "STUN", text)?;
    }
    if report.sockets.is_empty() {
        return writeln!(
            output,
            "\n{}",
            term.paint(Role::Dim, "No Native UDP socket observes its mapping.")
        );
    }
    writeln!(output)?;
    let mut table = Table::new(vec![
        "SOCKET", "BOUND", "MAPPED", "OBSERVER", "PORT", "MAPPING",
    ]);
    for socket in &report.sockets {
        let resource = resource(&socket.resource, peers);
        let port = match socket.port_mapping {
            PortMapping::Preserved => Cell::styled("preserved", Role::Good),
            PortMapping::Changed => Cell::styled("changed", Role::Warn),
            PortMapping::Unknown => Cell::styled("unknown", Role::Dim),
        };
        let mapping = match socket.endpoint_mapping {
            EndpointMapping::Independent => Cell::styled("endpoint-independent", Role::Good),
            EndpointMapping::Dependent => Cell::styled("endpoint-dependent", Role::Warn),
            EndpointMapping::Unknown => Cell::styled("unknown", Role::Dim),
        };
        let mut mapped: Vec<String> = Vec::new();
        let mut observers: Vec<String> = Vec::new();
        for mapping in &socket.mappings {
            mapped.push(if mapping.fresh {
                mapping.mapped.to_string()
            } else {
                format!("{} (earlier)", mapping.mapped)
            });
            observers.push(mapping.observer.to_string());
        }
        if mapped.is_empty() {
            mapped.push("—".into());
            observers.push("—".into());
        }
        table.row(vec![
            Cell::new(resource),
            Cell::new(socket.bound_address.to_string()),
            Cell::styled(mapped.join(", "), Role::Accent),
            Cell::new(observers.join(", ")),
            port,
            mapping,
        ]);
    }
    table.render(output, term)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SHA: &str = "00000000-0000-4000-8000-000000000003";

    fn report() -> NetcheckReport {
        serde_json::from_value(json!({
            "observedAt":"2026-10-11T00:00:00Z",
            "server":{"origin":"https://server.example/","reachable":true,"rttUs":80150},
            "unavailable":null,"ipv4":true,"ipv6":false,"udp":true,
            "sockets":[
                {"resource":{"kind":"listener","key":"lab-udp"},"boundAddress":"0.0.0.0:35000",
                 "mappings":[{"observer":"192.0.2.10:3478","mapped":"198.51.100.4:36928","rttUs":38970,"fresh":true}],
                 "portMapping":"changed","endpointMapping":"unknown"},
                {"resource":{"kind":"dial","key":format!("{SHA}/lab-udp")},"boundAddress":"[::]:40651",
                 "mappings":[],"portMapping":"unknown","endpointMapping":"unknown"}],
            "observers":[{"observer":"192.0.2.10:3478","rttUs":38970,
                "sizeProbe":{"size":1500,"answered":true},"pathMtu":1500}]
        }))
        .unwrap()
    }

    #[test]
    fn plain_report_names_dial_peers_and_shows_absence_as_unknown() {
        let peer: meshway_protocol::ipc::PeerStatus = serde_json::from_value(json!({
            "nodeId":SHA,"hostname":"meshway-sha","addresses":{"ipv4":null,"ipv6":null},
            "state":"idle","transport":null,"endpoint":null,"mtu":null,"via":[],"rtt":null
        }))
        .unwrap();
        let mut output = Vec::new();
        write(&mut output, &Term::plain(), &report(), &[peer]).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("  Server     https://server.example/ · reachable in 80.15 ms\n"));
        assert!(text.contains(
            "  STUN       192.0.2.10:3478 in 38.97 ms · 1500 B DF answered · route MTU 1500\n"
        ));
        let listener = text
            .lines()
            .find(|line| line.starts_with("listener lab-udp "))
            .unwrap();
        let fields: Vec<&str> = listener.split_whitespace().collect();
        assert_eq!(
            fields[2..],
            [
                "0.0.0.0:35000",
                "198.51.100.4:36928",
                "192.0.2.10:3478",
                "changed",
                "unknown"
            ]
        );
        assert!(text.contains("dial meshway-sha/lab-udp"));
        assert!(text.lines().last().unwrap().ends_with("unknown  unknown"));
    }

    #[test]
    fn a_node_without_a_packet_plane_reports_only_the_server() {
        let mut report = report();
        report.unavailable = Some(meshway_protocol::ipc::NetcheckUnavailable::NotRunning);
        let mut output = Vec::new();
        write(&mut output, &Term::plain(), &report, &[]).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("Network    not checked (not running)"));
        assert!(!text.contains("SOCKET"));
    }
}
