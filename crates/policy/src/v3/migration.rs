//! Conservative symbolic migration: never emits an installable partial policy.
use super::*;
use meshway_protocol::{
    AccessAction, DefaultRouteAction, DestinationScope, DestinationSelectorKind, MeshAccessPolicy,
    RouteActionKind, RoutingPolicy,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration<T> {
    pub converted: Option<T>,
    pub diagnostics: Vec<Diagnostic>,
}
fn problem(d: &mut Vec<Diagnostic>, field: impl Into<String>, message: impl Into<String>) {
    d.push(Diagnostic {
        field: field.into(),
        message: message.into(),
    });
}
fn selectors_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|a| {
        b.iter().any(|b| {
            a == "*" || b == "*" || a == b || a.starts_with("tag:") || b.starts_with("tag:")
        })
    })
}
fn access_overlap(
    a: &meshway_protocol::MeshAccessRule,
    b: &meshway_protocol::MeshAccessRule,
) -> bool {
    (selectors_overlap(&a.source, &b.source) && selectors_overlap(&a.destination, &b.destination))
        || ((a.bidirectional || b.bidirectional)
            && selectors_overlap(&a.source, &b.destination)
            && selectors_overlap(&a.destination, &b.source))
}
pub fn migrate_access(old: &MeshAccessPolicy) -> Migration<Vec<Grant>> {
    let mut d = vec![];
    for (i, r) in old.rules.iter().enumerate() {
        for (field, selectors) in [("source", &r.source), ("destination", &r.destination)] {
            if selectors.is_empty()
                || selectors.len() > 32
                || selectors.iter().any(|s| {
                    s != "*"
                        && !crate::valid_tag_selector(s)
                        && !s
                            .strip_prefix("node:")
                            .is_some_and(super::validation::valid_id)
                })
            {
                problem(
                    &mut d,
                    format!("rules[{i}].{field}"),
                    "Invalid or unsupported v2 selector list",
                );
            }
        }
        if r.action == AccessAction::Reject
            && (old.default_action == AccessAction::Allow
                || old
                    .rules
                    .iter()
                    .skip(i + 1)
                    .any(|a| a.action == AccessAction::Allow && access_overlap(r, a)))
        {
            problem(
                &mut d,
                format!("rules[{i}].action"),
                format!(
                    "Reject exception {} cannot be represented by additive grants; retain v2 until rewritten",
                    r.id
                ),
            );
        }
    }
    let mut grants = if old.default_action == AccessAction::Allow {
        vec![Grant {
            id: "migrated-default-members".into(),
            src: vec!["*".into()],
            dst: vec!["*".into()],
            bidirectional: false,
        }]
    } else {
        old.rules
            .iter()
            .filter(|r| r.action == AccessAction::Allow)
            .map(|r| Grant {
                id: r.id.clone(),
                src: r.source.clone(),
                dst: r.destination.clone(),
                bidirectional: r.bidirectional,
            })
            .collect()
    };
    grants.sort_by(|a, b| a.id.cmp(&b.id));
    Migration {
        converted: d.is_empty().then_some(grants),
        diagnostics: d,
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigratedRouting {
    pub mesh: Vec<MeshRule>,
    pub exits: Vec<ExitRule>,
}
pub fn migrate_routing(old: &RoutingPolicy) -> Migration<MigratedRouting> {
    let mut d = vec![];
    let mut out = MigratedRouting::default();
    if old.default_action == DefaultRouteAction::Reject {
        problem(
            &mut d,
            "default_action",
            "Default reject is not representable by v3 routing (routing cannot revoke grants)",
        );
    }
    for (i, r) in old.rules.iter().enumerate() {
        let base = format!("rules[{i}]");
        if r.source
            .node_ids
            .iter()
            .chain(&r.action.via)
            .chain(r.action.exit_node_id.iter())
            .any(|s| !super::validation::valid_id(s))
        {
            problem(&mut d, base.clone(), "Invalid v2 node identity");
        }
        if !r.source.countries.is_empty() {
            problem(
                &mut d,
                format!("{base}.source.countries"),
                "Country selectors unsupported; node expansion is not equivalent for future members",
            );
        }
        if r.action.kind == RouteActionKind::Reject {
            problem(
                &mut d,
                format!("{base}.action"),
                "Routing reject cannot become a grant or an automatic route",
            );
            continue;
        }
        let sources = if r.source.node_ids.is_empty() {
            vec!["*".into()]
        } else {
            r.source
                .node_ids
                .iter()
                .map(|s| format!("node:{s}"))
                .collect()
        };
        let via: Vec<_> = r.action.via.iter().map(|s| format!("node:{s}")).collect();
        match r.destination_scope {
            DestinationScope::Mesh => {
                if r.action.exit_node_id.is_some() {
                    problem(
                        &mut d,
                        format!("{base}.action.exit_node_id"),
                        "Mesh route cannot have exit",
                    );
                }
                let destinations = if r.destination_any.is_empty() {
                    vec!["*".into()]
                } else {
                    r.destination_any.iter().filter_map(|s| {
                    if s.kind==DestinationSelectorKind::Node { Some(format!("node:{}",s.value)) } else { problem(&mut d,format!("{base}.destination_any"),"Mesh domain/IP/country predicates not representable by endpoint selectors");None }
                }).collect()
                };
                for (a, src) in sources.iter().enumerate() {
                    for (b, dst) in destinations.iter().enumerate() {
                        out.mesh.push(MeshRule {
                            id: format!("migrated-mesh-{i}-{a}-{b}"),
                            src: src.clone(),
                            dst: dst.clone(),
                            via: via.clone(),
                        });
                    }
                }
            }
            DestinationScope::Internet => {
                let target = if r.action.exit_node_id.is_some() {
                    ExitTarget::Nodes
                } else {
                    ExitTarget::Direct
                };
                if target == ExitTarget::Direct && !via.is_empty() {
                    problem(
                        &mut d,
                        format!("{base}.action.via"),
                        "Direct action cannot preserve nonempty via",
                    );
                }
                let destinations = if r.destination_any.is_empty() {
                    problem(
                        &mut d,
                        format!("{base}.destination_any"),
                        "Wildcard Internet predicate also matches domain-only context; CIDR defaults require a resolved IP, so conversion needs explicit review",
                    );
                    vec![
                        ExitMatch::Cidr("0.0.0.0/0".into()),
                        ExitMatch::Cidr("::/0".into()),
                    ]
                } else {
                    r.destination_any
                        .iter()
                        .filter_map(|s| match s.kind {
                            DestinationSelectorKind::DomainSuffix => {
                                meshway_protocol::normalize_dns_name(&s.value)
                                    .map(ExitMatch::DomainSuffix)
                                    .map_err(|e| {
                                        problem(&mut d, format!("{base}.destination_any"), e)
                                    })
                                    .ok()
                            }
                            DestinationSelectorKind::Cidr => s
                                .value
                                .parse::<ipnet::IpNet>()
                                .map(|n| ExitMatch::Cidr(n.trunc().to_string()))
                                .map_err(|_| {
                                    problem(
                                        &mut d,
                                        format!("{base}.destination_any"),
                                        "Invalid CIDR",
                                    )
                                })
                                .ok(),
                            _ => {
                                problem(
                                    &mut d,
                                    format!("{base}.destination_any"),
                                    "Internet node/country selector unsupported",
                                );
                                None
                            }
                        })
                        .collect()
                };
                for (a, src) in sources.iter().enumerate() {
                    for (b, destination) in destinations.iter().enumerate() {
                        out.exits.push(ExitRule {
                            id: format!("migrated-exit-{i}-{a}-{b}"),
                            src: src.clone(),
                            destinations: vec![destination.clone()],
                            target,
                            candidates: r
                                .action
                                .exit_node_id
                                .iter()
                                .map(|exit| format!("node:{exit}"))
                                .collect(),
                            via: via.clone(),
                        });
                    }
                }
            }
            DestinationScope::Any => problem(
                &mut d,
                format!("{base}.destination_scope"),
                "Any scope requires separate mesh/internet semantic review",
            ),
        }
    }
    // Overlap with different actions requires first-match subtraction. Even if
    // one ordering happens to match v3 specificity, do not guess equivalence.
    for (i, a) in out.mesh.iter().enumerate() {
        for b in out.mesh.iter().skip(i + 1) {
            if a.via != b.via
                && selectors_overlap(std::slice::from_ref(&a.src), std::slice::from_ref(&b.src))
                && selectors_overlap(std::slice::from_ref(&a.dst), std::slice::from_ref(&b.dst))
            {
                problem(
                    &mut d,
                    "rules",
                    format!(
                        "Order-dependent overlap {} / {} requires predicate subtraction",
                        a.id, b.id
                    ),
                );
            }
        }
    }
    out.mesh.sort_by(|a, b| a.id.cmp(&b.id));
    Migration {
        converted: d.is_empty().then_some(out),
        diagnostics: d,
    }
}
