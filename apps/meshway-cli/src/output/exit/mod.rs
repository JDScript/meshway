//! `meshway exit`: this Node's routing profile selection as its netmap
//! decides it, and the allowed profiles with what each does.
use super::table::{Cell, Table};
use super::term::{Role, Term, pad};
use meshway_configuration::RouteAction;
use meshway_configuration::selection::{
    AutoBasis, ProfileKind, ProfileSummary, RoutingSelectionState, SelectionUnavailable,
};
use meshway_protocol::ipc::PeerStatus;
use std::io::{self, Write};

fn line(output: &mut impl Write, term: &Term, label: &str, value: String) -> io::Result<()> {
    writeln!(
        output,
        "  {} {value}",
        term.paint(Role::Dim, pad(label, 10))
    )
}

/// Names Node IDs after the installed netmap's peers; unknown IDs stay IDs.
pub(crate) struct Names<'a>(pub &'a [PeerStatus]);

impl Names<'_> {
    fn node(&self, id: &meshway_model::NodeId) -> String {
        self.0
            .iter()
            .find(|peer| peer.node_id == *id)
            .map(|peer| peer.hostname.clone())
            .unwrap_or_else(|| id.to_string())
    }
    fn selectors(&self, selectors: &[String]) -> String {
        selectors
            .iter()
            .map(|selector| {
                selector
                    .strip_prefix("node:")
                    .and_then(|id| id.parse().ok())
                    .map(|id| self.node(&id))
                    .unwrap_or_else(|| selector.clone())
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn action(&self, action: &RouteAction) -> String {
        match action {
            RouteAction::Exit { nodes, via } if via.is_empty() => {
                format!("exit {}", self.selectors(nodes))
            }
            RouteAction::Exit { nodes, via } => {
                format!("exit {} via {}", self.selectors(nodes), self.selectors(via))
            }
            RouteAction::Direct => "direct".into(),
            RouteAction::Unmanaged => "unmanaged".into(),
        }
    }
}

/// One sentence of what the profile does after the public rules.
pub(crate) fn summary(profile: &ProfileSummary, names: &Names<'_>) -> String {
    match profile.kind {
        ProfileKind::Mesh => "Mesh only; other external traffic unmanaged".into(),
        ProfileKind::Global => match &profile.fallback {
            Some(action) => format!("all external traffic: {}", names.action(action)),
            None => "all external traffic through one exit".into(),
        },
        ProfileKind::Auto => "the allowed profile the Node's region selects".into(),
        ProfileKind::Custom => {
            let mut parts = vec![match profile.rules {
                1 => "1 rule".to_owned(),
                n => format!("{n} rules"),
            }];
            if !profile.exits.is_empty() {
                parts.push(format!(
                    "exits {}",
                    profile
                        .exits
                        .iter()
                        .map(|id| names.node(id))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if let Some(fallback) = &profile.fallback {
                parts.push(format!("otherwise {}", names.action(fallback)));
            }
            parts.join(" · ")
        }
    }
}

fn kind(kind: ProfileKind) -> &'static str {
    match kind {
        ProfileKind::Mesh => "mesh",
        ProfileKind::Global => "global",
        ProfileKind::Auto => "auto",
        ProfileKind::Custom => "custom",
    }
}

pub(crate) fn write(
    output: &mut impl Write,
    term: &Term,
    selection: &RoutingSelectionState,
    names: &Names<'_>,
) -> io::Result<()> {
    let mut title = format!(
        "{} {}",
        term.paint(Role::Bold, "Exit profile"),
        term.paint(Role::Good, &selection.applied_profile)
    );
    if selection.locked {
        title.push_str(&term.paint(Role::Dim, " (locked)"));
    }
    writeln!(output, "{title}")?;
    let requested = match (
        &selection.requested_profile,
        selection.requested_unavailable,
    ) {
        (None, _) => term.paint(Role::Dim, "none · following the default"),
        (Some(profile), None) => format!(
            "{profile} · version {}",
            selection.version.map(|v| v.get()).unwrap_or_default()
        ),
        (Some(profile), Some(reason)) => term.paint(
            Role::Warn,
            format!(
                "{profile} · not applied ({})",
                match reason {
                    SelectionUnavailable::NotAllowed => "no longer allowed",
                    SelectionUnavailable::Locked => "locked to the default",
                }
            ),
        ),
    };
    line(output, term, "Requested", requested)?;
    line(
        output,
        term,
        "Default",
        format!(
            "{} · {}",
            selection.default_profile,
            if selection.locked {
                "locked by an administrator"
            } else {
                "not locked"
            }
        ),
    )?;
    if let Some(auto) = &selection.auto {
        line(
            output,
            term,
            "Auto",
            format!(
                "auto → {} (region {}, basis {})",
                auto.profile,
                auto.region.as_deref().unwrap_or("none"),
                match auto.basis {
                    AutoBasis::Label => "label",
                    AutoBasis::NoRuleMatched => "noRuleMatched",
                    AutoBasis::NoTrustedRegion => "noTrustedRegion",
                }
            ),
        )?;
    }
    writeln!(output)?;
    let mut table = Table::new(vec!["PROFILE", "KIND", "STATE", "SUMMARY"]);
    for profile in &selection.profiles {
        let mut state = Vec::new();
        if profile.name == selection.applied_profile {
            state.push("applied");
        }
        if selection.requested_profile.as_deref() == Some(profile.name.as_str())
            && profile.name != selection.applied_profile
        {
            state.push("requested");
        }
        if profile.name == selection.default_profile {
            state.push("default");
        }
        let applied = profile.name == selection.applied_profile;
        table.row(vec![
            Cell::styled(
                &profile.name,
                if applied { Role::Good } else { Role::Plain },
            ),
            Cell::new(kind(profile.kind)),
            Cell::styled(state.join(", "), Role::Dim),
            Cell::new(summary(profile, names)),
        ]);
    }
    table.render(output, term)
}

/// The note after `meshway exit --default`.
pub(crate) fn cleared(term: &Term, selection: &RoutingSelectionState) -> String {
    term.paint(
        Role::Dim,
        format!(
            "Persistent selection cleared; following the default `{}`.",
            selection.default_profile
        ),
    )
}

#[cfg(test)]
mod tests;
