//! The decision line and echo lines of a target outside the Mesh.
use super::Ping;
use crate::output::format::{millis, words};
use crate::output::term::{Role, Term};
use meshway_model::NodeId;
use meshway_protocol::ipc::{
    PingAction, PingDecision, PingEcho, PingEchoOutcome, PingEvent, PingIcmpError, PingResolver,
    PingRule,
};
use std::io::{self, Write};

impl Ping {
    fn path(&self, path: &[NodeId]) -> String {
        path.iter()
            .map(|id| self.name(id))
            .collect::<Vec<_>>()
            .join(" → ")
    }

    /// What the decided target is called: the name asked for, else the address.
    fn label(&self) -> String {
        self.decision
            .as_ref()
            .and_then(|decision| {
                decision
                    .resolution
                    .as_ref()
                    .map(|resolution| resolution.name.clone())
                    .or(decision.destination.clone())
            })
            .unwrap_or_else(|| "ping".into())
    }

    pub(super) fn decision(
        &mut self,
        output: &mut impl Write,
        term: &Term,
        decision: &PingDecision,
    ) -> io::Result<()> {
        if let Some(resolution) = &decision.resolution {
            let resolver = match &resolution.resolver {
                PingResolver::MagicDns => "MagicDNS".to_owned(),
                PingResolver::Exit { node } => format!("exit {}", self.name(node)),
                PingResolver::Resolvers { keys } => format!("resolver {}", keys.join(", ")),
                PingResolver::System => "system DNS".to_owned(),
                PingResolver::Unavailable => "no DNS path".to_owned(),
            };
            let mut text = if resolution.addresses.is_empty() {
                format!("{} did not resolve via {resolver}", resolution.name)
            } else {
                format!(
                    "{} resolved via {resolver}: {}",
                    resolution.name,
                    resolution.addresses.join(", ")
                )
            };
            if let Some(association) = &resolution.association {
                text.push_str(&format!(
                    " · associated as {}, {} s left",
                    association.names.join(", "),
                    association.remaining_seconds
                ));
            }
            writeln!(output, "{}", term.paint(Role::Dim, text))?;
        }
        let Some(action) = &decision.action else {
            return Ok(());
        };
        // A Mesh Node target keeps its measurement lines; its decision only
        // shows with -v or when it is not probed.
        let denied = matches!(action, PingAction::Denied { .. });
        if decision.node.is_some() && !self.verbose && !denied {
            return Ok(());
        }
        if decision.node.is_none() {
            self.decision = Some(decision.clone());
        }
        let destination = match (&decision.node, &decision.destination) {
            (Some(node), _) => node.hostname.clone(),
            (None, Some(destination)) => destination.clone(),
            (None, None) => return Ok(()),
        };
        let (role, what) = match action {
            PingAction::Mesh => (
                Role::Plain,
                format!("Mesh via {}", self.path(&decision.path)),
            ),
            PingAction::Exit { node } => (
                Role::Plain,
                format!("exit {} via {}", self.name(node), self.path(&decision.path)),
            ),
            PingAction::Direct => (Role::Plain, "direct, not via Mesh".to_owned()),
            PingAction::Unmanaged => (Role::Plain, "unmanaged, not via Mesh".to_owned()),
            PingAction::Denied { reason } => {
                (Role::Bad, format!("denied ({}), not probed", words(reason)))
            }
        };
        let mut text = format!("decision for {destination}: {what}");
        match (&decision.profile, &decision.rule) {
            (profile, Some(rule)) => {
                text.push_str(" · ");
                if let Some(profile) = profile {
                    text.push_str(&format!("profile {profile}, "));
                }
                text.push_str(&rule_text(rule));
            }
            (Some(profile), None) => text.push_str(&format!(" · profile {profile}, fallback")),
            (None, None) => {}
        }
        writeln!(output, "{}", term.paint(role, text))
    }

    pub(super) fn echo(
        &mut self,
        output: &mut impl Write,
        term: &Term,
        echo: &PingEcho,
    ) -> io::Result<()> {
        let destination = self
            .decision
            .as_ref()
            .and_then(|decision| decision.destination.clone())
            .unwrap_or_else(|| "target".into());
        let route = match self.decision.as_ref().and_then(|d| d.action.as_ref()) {
            Some(PingAction::Exit { node }) => {
                let path = if echo.path.is_empty() {
                    self.decision
                        .as_ref()
                        .map(|d| d.path.clone())
                        .unwrap_or_default()
                } else {
                    echo.path.clone()
                };
                let exit = path.last().copied().unwrap_or(*node);
                format!("via exit {} ({})", self.name(&exit), self.path(&path))
            }
            Some(PingAction::Unmanaged) => "unmanaged, not via Mesh".to_owned(),
            _ => "direct, not via Mesh".to_owned(),
        };
        let from = echo.from.clone().unwrap_or_else(|| destination.clone());
        let rtt = match echo.duration_us {
            Some(duration) => format!("in {}", term.paint(Role::Bold, millis(duration.get()))),
            None => "with no measured round trip".into(),
        };
        let setup = if echo.setup {
            " (includes circuit setup)"
        } else {
            ""
        };
        match echo.outcome {
            PingEchoOutcome::Reply => {
                if let Some(duration) = echo.duration_us {
                    self.rtts.push(duration.get());
                }
                writeln!(
                    output,
                    "{} from {from} {route} {rtt}{setup}",
                    term.paint(Role::Good, "pong")
                )
            }
            PingEchoOutcome::Unreachable => {
                self.unreachable += 1;
                let error = echo.error.map(icmp).unwrap_or_else(|| "ICMP error".into());
                writeln!(
                    output,
                    "{}",
                    term.paint(
                        Role::Warn,
                        format!("unreachable from {from} ({error}) {route} {rtt}{setup}")
                    )
                )
            }
            PingEchoOutcome::Timeout => writeln!(
                output,
                "{}",
                term.paint(
                    Role::Warn,
                    format!("no reply from {destination} {route} within 5 s")
                )
            ),
            PingEchoOutcome::Failed => writeln!(
                output,
                "{}",
                term.paint(Role::Bad, format!("echo to {destination} failed locally"))
            ),
        }
    }

    pub(super) fn external_summary(
        &mut self,
        output: &mut impl Write,
        term: &Term,
        event: &PingEvent,
    ) -> io::Result<()> {
        let PingEvent::Completed {
            sent,
            success,
            timeout,
            failed,
        } = event
        else {
            return Ok(());
        };
        let loss = if *sent == 0 {
            0
        } else {
            u64::from(*timeout) * 100 / u64::from(*sent)
        };
        let mut text = format!(
            "{}: {sent} sent, {success} replied, {loss}% loss",
            self.label()
        );
        if self.unreachable > 0 {
            text.push_str(&format!(", {} unreachable", self.unreachable));
        }
        if *failed > 0 {
            text.push_str(&format!(", {failed} failed locally"));
        }
        text.push_str(&self.rtt_summary());
        writeln!(output, "{}", term.paint(Role::Bold, text))
    }
}

fn rule_text(rule: &PingRule) -> String {
    let mut text = format!("rule {}", rule.pointer);
    if !rule.remark.is_empty() {
        text.push_str(&format!(" \"{}\"", rule.remark));
    }
    if let Some(set) = &rule.rule_set {
        text.push_str(&format!(" (rule set {}: {})", set.key, set.entry));
    }
    text
}

fn icmp(error: PingIcmpError) -> String {
    match (error.kind, error.code) {
        (3, 0) => "network unreachable".into(),
        (3, 1) => "host unreachable".into(),
        (3, 2) => "protocol unreachable".into(),
        (3, 3) => "port unreachable".into(),
        (3, 4) => "fragmentation needed".into(),
        (3, 9 | 10 | 13) => "administratively prohibited".into(),
        (11, 0) => "TTL exceeded".into(),
        (11, 1) => "reassembly time exceeded".into(),
        (kind, code) => format!("ICMP type {kind} code {code}"),
    }
}
