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

pub(crate) fn write(
    output: &mut impl Write,
    term: &Term,
    report: &NetcheckReport,
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
        line(
            output,
            term,
            "STUN",
            format!(
                "{} {}",
                observer.observer,
                observer
                    .rtt_us
                    .map(|rtt| format!("in {}", millis(rtt.get())))
                    .unwrap_or_else(|| "(round trip ambiguous)".into())
            ),
        )?;
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
        let resource = match &socket.resource {
            ResourceRef::Listener { key } => format!("listener {key}"),
            ResourceRef::Dial { key } => format!("dial {key}"),
            ResourceRef::Coordination { key } => format!("coordination {key}"),
        };
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
