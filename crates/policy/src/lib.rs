//! Pure, deterministic compilation and preview of logical routing intent.
//!
//! This crate performs no I/O, DNS queries, GeoIP lookups, packet handling, or
//! transport selection. Inventory must contain only active nodes. Successful
//! compilation verifies neither adjacency nor transport capability, ACL grants,
//! or traffic reachability. A routing decision NEVER grants network access.
//! Rule order and mandatory hop order are preserved, including when a resolved
//! path is invalid: preview fails closed rather than skipping a hop.

pub mod access;
pub use access::{compile_mesh_paths, validate_mesh_access};

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};
use ipnet::IpNet;
use meshway_protocol::routing::{
    DefaultRouteAction, DestinationScope, DestinationSelector, DestinationSelectorKind, FlowQuery,
    MeshNetwork, MeshNode, PolicyDiagnostic, RouteAction, RouteActionKind, RouteDecision,
    RouteDecisionAction, RouteRule, RoutingPolicy,
};

pub const MAX_RULES: usize = 128;
pub const MAX_VIA: usize = 8;
/// Total entries across source node IDs, source countries and destination_any.
pub const MAX_SELECTORS_PER_RULE: usize = 32;
pub const MAX_POLICY_BYTES: usize = 128 * 1024;
pub const MAX_DIAGNOSTICS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileReport {
    pub normalized_policy: RoutingPolicy,
    pub diagnostics: Vec<PolicyDiagnostic>,
}

impl CompileReport {
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewResult {
    /// True includes intentional reject decisions. Invalid policy, inventory,
    /// flow context or resolved path yields false and an `invalid` decision.
    pub valid: bool,
    pub diagnostics: Vec<PolicyDiagnostic>,
    pub decision: RouteDecision,
}

/// Normalize policy values and validate all references against active inventory.
/// Inputs are never modified. Invalid output must not be installed as a policy.
pub fn compile(policy: &RoutingPolicy, network: &MeshNetwork, nodes: &[MeshNode]) -> CompileReport {
    prepare(policy, network, nodes).0
}

/// Resolve trusted mesh context, then evaluate the first matching rule. Internet
/// country/domain inputs are explicit preview hints, not verified DNS or GeoIP.
pub fn preview(
    policy: &RoutingPolicy,
    network: &MeshNetwork,
    nodes: &[MeshNode],
    flow: &FlowQuery,
) -> PreviewResult {
    let (report, inventory) = prepare(policy, network, nodes);
    if !report.is_valid() {
        return PreviewResult {
            valid: false,
            diagnostics: report.diagnostics,
            decision: empty_decision(
                RouteDecisionAction::Invalid,
                None,
                "Policy or active-node inventory is invalid",
            ),
        };
    }

    let context = match resolve_flow(flow, &inventory) {
        Ok(context) => context,
        Err((field, reason)) => return invalid_preview(None, field, reason),
    };
    for rule in &report.normalized_policy.rules {
        if !matches_rule(rule, &context) {
            continue;
        }
        if rule.action.kind == RouteActionKind::Reject {
            return valid_preview(empty_decision(
                RouteDecisionAction::Reject,
                Some(&rule.id),
                "Matched reject rule",
            ));
        }
        return resolve_path(
            &context,
            &rule.action,
            Some(&rule.id),
            "Matched route rule; logical intent only",
        );
    }

    match report.normalized_policy.default_action {
        DefaultRouteAction::Reject => valid_preview(empty_decision(
            RouteDecisionAction::Reject,
            None,
            "No rule matched; default reject",
        )),
        DefaultRouteAction::Direct => resolve_path(
            &context,
            &RouteAction {
                kind: RouteActionKind::Route,
                via: vec![],
                exit_node_id: None,
            },
            None,
            "No rule matched; default direct logical path",
        ),
    }
}

fn prepare<'a>(
    policy: &RoutingPolicy,
    network: &MeshNetwork,
    nodes: &'a [MeshNode],
) -> (CompileReport, Inventory<'a>) {
    let mut report = CompileReport {
        normalized_policy: policy.clone(),
        diagnostics: vec![],
    };
    let inventory = Inventory::validate(network, nodes, &mut report.diagnostics);
    if !policy_fits(policy) {
        diagnostic(
            &mut report.diagnostics,
            None,
            "policy",
            "Serialized policy exceeds the 128 KiB limit",
        );
        return (report, inventory);
    }
    if policy.rules.len() > MAX_RULES {
        diagnostic(
            &mut report.diagnostics,
            None,
            "rules",
            "Policy exceeds the 128-rule limit",
        );
    }

    let mut rule_ids = BTreeSet::new();
    for rule in &mut report.normalized_policy.rules {
        if rule.id.trim().is_empty() {
            diagnostic(
                &mut report.diagnostics,
                Some(&rule.id),
                "id",
                "Rule ID must not be empty or whitespace",
            );
        }
        if !rule_ids.insert(rule.id.clone()) {
            diagnostic(
                &mut report.diagnostics,
                Some(&rule.id),
                "id",
                "Rule IDs must be unique",
            );
        }
        validate_rule(rule, &inventory, &mut report.diagnostics);
    }
    // IDNA can expand Unicode labels. Bound the installed representation too.
    if !policy_fits(&report.normalized_policy) {
        diagnostic(
            &mut report.diagnostics,
            None,
            "policy",
            "Normalized serialized policy exceeds the 128 KiB limit",
        );
    }
    (report, inventory)
}

fn validate_rule(
    rule: &mut RouteRule,
    inventory: &Inventory<'_>,
    diagnostics: &mut Vec<PolicyDiagnostic>,
) {
    let id = Some(rule.id.as_str());
    let selector_count =
        rule.source.node_ids.len() + rule.source.countries.len() + rule.destination_any.len();
    if selector_count > MAX_SELECTORS_PER_RULE {
        diagnostic(
            diagnostics,
            id,
            "selectors",
            "A rule allows at most 32 total source and destination selector entries",
        );
    }
    for (index, node_id) in rule.source.node_ids.iter().enumerate() {
        inventory.check_reference(
            node_id,
            id,
            &format!("source.node_ids[{index}]"),
            diagnostics,
        );
    }
    for (index, country) in rule.source.countries.iter_mut().enumerate() {
        normalize_country_field(
            country,
            id,
            &format!("source.countries[{index}]"),
            diagnostics,
        );
    }
    for (index, selector) in rule.destination_any.iter_mut().enumerate() {
        let field = format!("destination_any[{index}].value");
        match selector.kind {
            DestinationSelectorKind::Node => {
                inventory.check_reference(&selector.value, id, &field, diagnostics);
                if rule.destination_scope == DestinationScope::Internet {
                    diagnostic(
                        diagnostics,
                        id,
                        field,
                        "Node selectors require mesh or any destination scope",
                    );
                }
            }
            DestinationSelectorKind::DomainSuffix => match normalize_domain(&selector.value) {
                Ok(domain) => selector.value = domain,
                Err(reason) => diagnostic(diagnostics, id, field, reason),
            },
            DestinationSelectorKind::Cidr => match selector.value.parse::<IpNet>() {
                Ok(prefix) => selector.value = prefix.trunc().to_string(),
                Err(_) => diagnostic(diagnostics, id, field, "Invalid IPv4 or IPv6 CIDR prefix"),
            },
            DestinationSelectorKind::Country => {
                normalize_country_field(&mut selector.value, id, &field, diagnostics);
            }
        }
    }

    if rule.action.via.len() > MAX_VIA {
        diagnostic(
            diagnostics,
            id,
            "action.via",
            "A route allows at most 8 via nodes",
        );
    }
    let mut via_ids = BTreeSet::new();
    for (index, node_id) in rule.action.via.iter().enumerate() {
        let field = format!("action.via[{index}]");
        inventory.check_reference(node_id, id, &field, diagnostics);
        if !via_ids.insert(node_id.as_str()) {
            diagnostic(diagnostics, id, field, "Via nodes must not repeat");
        }
    }
    if let Some(exit_id) = &rule.action.exit_node_id {
        inventory.check_reference(exit_id, id, "action.exit_node_id", diagnostics);
        if via_ids.contains(exit_id.as_str()) {
            diagnostic(
                diagnostics,
                id,
                "action.exit_node_id",
                "Exit node must not also appear in via nodes",
            );
        }
        if rule.destination_scope != DestinationScope::Internet {
            diagnostic(
                diagnostics,
                id,
                "action.exit_node_id",
                "An explicit exit requires internet destination scope",
            );
        }
    }
    match rule.action.kind {
        RouteActionKind::Reject => {
            if !rule.action.via.is_empty() || rule.action.exit_node_id.is_some() {
                diagnostic(
                    diagnostics,
                    id,
                    "action",
                    "Reject actions require empty via nodes and a null exit",
                );
            }
        }
        RouteActionKind::Route => {
            if rule.destination_scope != DestinationScope::Mesh
                && !rule.action.via.is_empty()
                && rule.action.exit_node_id.is_none()
            {
                diagnostic(
                    diagnostics,
                    id,
                    "action.exit_node_id",
                    "Internet-capable routes with via nodes require an explicit exit and internet scope",
                );
            }
        }
    }
}

struct InventoryNode<'a> {
    id: &'a str,
    country: Option<String>,
    ipv4: Ipv4Addr,
    ipv6: Ipv6Addr,
}

struct Inventory<'a> {
    nodes: BTreeMap<&'a str, InventoryNode<'a>>,
    addresses: BTreeMap<IpAddr, &'a str>,
    ipv4_pool: Option<IpNet>,
    ipv6_pool: Option<IpNet>,
}

impl<'a> Inventory<'a> {
    fn validate(
        network: &MeshNetwork,
        nodes: &'a [MeshNode],
        diagnostics: &mut Vec<PolicyDiagnostic>,
    ) -> Self {
        let mut inventory = Self {
            nodes: BTreeMap::new(),
            addresses: BTreeMap::new(),
            ipv4_pool: parse_pool(&network.ipv4_prefix, true, diagnostics),
            ipv6_pool: parse_pool(&network.ipv6_prefix, false, diagnostics),
        };
        let mut ids = BTreeSet::new();
        for (index, node) in nodes.iter().enumerate() {
            if node.id.trim().is_empty() || !ids.insert(node.id.as_str()) {
                diagnostic(
                    diagnostics,
                    None,
                    format!("nodes[{index}].id"),
                    "Active node IDs must be nonempty and unique",
                );
            }
            let country = node.country.as_deref().and_then(|value| {
                let normalized = normalize_country(value);
                if normalized.is_none() {
                    diagnostic(
                        diagnostics,
                        None,
                        format!("nodes[{index}].country"),
                        "Country must be two ASCII letters or null",
                    );
                }
                normalized
            });
            let ipv4 = node.mesh_ipv4.parse::<Ipv4Addr>().ok();
            let ipv6 = node.mesh_ipv6.parse::<Ipv6Addr>().ok();
            for (ip, pool, suffix) in [
                (ipv4.map(IpAddr::V4), inventory.ipv4_pool, "mesh_ipv4"),
                (ipv6.map(IpAddr::V6), inventory.ipv6_pool, "mesh_ipv6"),
            ] {
                let field = format!("nodes[{index}].{suffix}");
                if let Some(ip) = ip {
                    if pool.is_some_and(|prefix| !prefix.contains(&ip)) {
                        diagnostic(
                            diagnostics,
                            None,
                            &field,
                            "Node address is outside its configured mesh pool",
                        );
                    }
                    if inventory.addresses.insert(ip, &node.id).is_some() {
                        diagnostic(
                            diagnostics,
                            None,
                            field,
                            "Active node addresses must be unique",
                        );
                    }
                } else {
                    diagnostic(
                        diagnostics,
                        None,
                        field,
                        "Node address is malformed or has the wrong IP family",
                    );
                }
            }
            if let (Some(ipv4), Some(ipv6)) = (ipv4, ipv6) {
                inventory.nodes.insert(
                    &node.id,
                    InventoryNode {
                        id: &node.id,
                        country,
                        ipv4,
                        ipv6,
                    },
                );
            }
        }
        inventory
    }

    fn check_reference(
        &self,
        node_id: &str,
        rule_id: Option<&str>,
        field: &str,
        diagnostics: &mut Vec<PolicyDiagnostic>,
    ) {
        if !self.nodes.contains_key(node_id) {
            diagnostic(
                diagnostics,
                rule_id,
                field,
                "Referenced node is not in the valid active inventory",
            );
        }
    }

    fn contains_ip(&self, ip: &IpAddr) -> bool {
        self.ipv4_pool.is_some_and(|prefix| prefix.contains(ip))
            || self.ipv6_pool.is_some_and(|prefix| prefix.contains(ip))
    }
}

fn parse_pool(value: &str, ipv4: bool, diagnostics: &mut Vec<PolicyDiagnostic>) -> Option<IpNet> {
    if let Ok(prefix) = value.parse::<IpNet>()
        && matches!(prefix, IpNet::V4(_)) == ipv4
    {
        return Some(prefix.trunc());
    }
    diagnostic(
        diagnostics,
        None,
        if ipv4 {
            "network.ipv4_prefix"
        } else {
            "network.ipv6_prefix"
        },
        "Mesh prefix is malformed or has the wrong IP family",
    );
    None
}

struct FlowContext<'a> {
    source: &'a InventoryNode<'a>,
    target: Option<&'a InventoryNode<'a>>,
    ip: Option<IpAddr>,
    domain: Option<String>,
    country: Option<String>,
}

type FlowError = (&'static str, &'static str);

fn resolve_flow<'a>(
    flow: &FlowQuery,
    inventory: &'a Inventory<'a>,
) -> Result<FlowContext<'a>, FlowError> {
    let source = inventory.nodes.get(flow.source_node_id.as_str()).ok_or((
        "flow.source_node_id",
        "Source node is not in the active inventory",
    ))?;
    if flow.destination_node_id.is_none() && flow.destination_ip.is_none() && flow.domain.is_none()
    {
        return Err((
            "flow.destination",
            "A destination node, literal IP, or domain is required; country alone is not a target",
        ));
    }
    let domain = flow
        .domain
        .as_deref()
        .map(normalize_domain)
        .transpose()
        .map_err(|reason| ("flow.domain", reason))?;
    let mut ip = flow
        .destination_ip
        .as_deref()
        .map(str::parse::<IpAddr>)
        .transpose()
        .map_err(|_| {
            (
                "flow.destination_ip",
                "Destination IP must be a literal IPv4 or IPv6 address",
            )
        })?;
    // Mapped aliases must never bypass IPv4 ownership, pool or policy checks.
    // Reject this unsupported form before either explicit-node or IP resolution.
    if let Some(IpAddr::V6(address)) = ip
        && address.to_ipv4_mapped().is_some()
    {
        return Err((
            "flow.destination_ip",
            "IPv4-mapped IPv6 destination addresses are unsupported; use an IPv4 literal",
        ));
    }
    let hint_country = flow
        .destination_country
        .as_deref()
        .map(|value| {
            normalize_country(value).ok_or((
                "flow.destination_country",
                "Destination country must be two ASCII letters or null",
            ))
        })
        .transpose()?;
    let target = if let Some(node_id) = &flow.destination_node_id {
        let node = inventory.nodes.get(node_id.as_str()).ok_or((
            "flow.destination_node_id",
            "Destination node is not in the active inventory",
        ))?;
        if ip.is_some_and(|address| {
            address != IpAddr::V4(node.ipv4) && address != IpAddr::V6(node.ipv6)
        }) {
            return Err((
                "flow.destination_ip",
                "Destination IP disagrees with the target node's assigned addresses",
            ));
        }
        ip = Some(ip.unwrap_or(IpAddr::V4(node.ipv4)));
        Some(node)
    } else if let Some(address) = ip {
        if let Some(node_id) = inventory.addresses.get(&address) {
            inventory.nodes.get(node_id)
        } else if inventory.contains_ip(&address) {
            return Err((
                "flow.destination_ip",
                "Destination is inside a mesh pool but has no active owner; internet fallback is forbidden",
            ));
        } else {
            None
        }
    } else {
        None
    };
    let country = if let Some(node) = target {
        if hint_country.is_some() && hint_country != node.country {
            return Err((
                "flow.destination_country",
                "Destination country disagrees with trusted mesh-node metadata",
            ));
        }
        node.country.clone()
    } else {
        hint_country
    };
    Ok(FlowContext {
        source,
        target,
        ip,
        domain,
        country,
    })
}

fn matches_rule(rule: &RouteRule, flow: &FlowContext<'_>) -> bool {
    (rule.source.node_ids.is_empty() || rule.source.node_ids.iter().any(|id| id == flow.source.id))
        && (rule.source.countries.is_empty()
            || flow
                .source
                .country
                .as_ref()
                .is_some_and(|country| rule.source.countries.contains(country)))
        && match rule.destination_scope {
            DestinationScope::Any => true,
            DestinationScope::Mesh => flow.target.is_some(),
            DestinationScope::Internet => flow.target.is_none(),
        }
        && (rule.destination_any.is_empty()
            || rule
                .destination_any
                .iter()
                .any(|selector| matches_destination(selector, flow)))
}

fn matches_destination(selector: &DestinationSelector, flow: &FlowContext<'_>) -> bool {
    match selector.kind {
        DestinationSelectorKind::Node => flow.target.is_some_and(|node| node.id == selector.value),
        DestinationSelectorKind::DomainSuffix => flow.domain.as_deref().is_some_and(|domain| {
            domain == selector.value
                || domain
                    .strip_suffix(selector.value.as_str())
                    .is_some_and(|prefix| prefix.ends_with('.'))
        }),
        DestinationSelectorKind::Cidr => flow.ip.is_some_and(|ip| {
            selector
                .value
                .parse::<IpNet>()
                .is_ok_and(|prefix| prefix.contains(&ip))
        }),
        DestinationSelectorKind::Country => flow.country.as_ref() == Some(&selector.value),
    }
}

fn resolve_path(
    flow: &FlowContext<'_>,
    action: &RouteAction,
    rule_id: Option<&str>,
    reason: &str,
) -> PreviewResult {
    let mut path = vec![flow.source.id.to_owned()];
    path.extend(action.via.iter().cloned());
    let exit_node_id = if let Some(target) = flow.target {
        if action.exit_node_id.is_some() {
            return invalid_preview(
                rule_id,
                "flow.path",
                "A mesh target cannot be redirected to an exit",
            );
        }
        path.push(target.id.to_owned());
        None
    } else if let Some(exit_id) = &action.exit_node_id {
        if exit_id != flow.source.id || !action.via.is_empty() {
            path.push(exit_id.clone());
        }
        Some(exit_id.clone())
    } else if action.via.is_empty() {
        Some(flow.source.id.to_owned())
    } else {
        return invalid_preview(
            rule_id,
            "flow.path",
            "Internet routes with via nodes require an explicit exit",
        );
    };
    let mut visited = BTreeSet::new();
    if path.iter().any(|node_id| !visited.insert(node_id)) {
        return invalid_preview(
            rule_id,
            "flow.path",
            "Resolved path repeats a source, via, target, or exit node; mandatory hops cannot be skipped",
        );
    }
    valid_preview(RouteDecision {
        action: RouteDecisionAction::Route,
        rule_id: rule_id.map(str::to_owned),
        path,
        exit_node_id,
        reason: reason.to_owned(),
    })
}

fn normalize_country(value: &str) -> Option<String> {
    (value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .then(|| value.to_ascii_uppercase())
}

fn normalize_country_field(
    value: &mut String,
    rule_id: Option<&str>,
    field: &str,
    diagnostics: &mut Vec<PolicyDiagnostic>,
) {
    match normalize_country(value) {
        Some(country) => *value = country,
        None => diagnostic(
            diagnostics,
            rule_id,
            field,
            "Country must be two ASCII letters",
        ),
    }
}

fn normalize_domain(value: &str) -> Result<String, &'static str> {
    let ascii = Uts46::new()
        .to_ascii(
            value.as_bytes(),
            AsciiDenyList::STD3,
            Hyphens::Check,
            DnsLength::VerifyAllowRootDot,
        )
        .map_err(|_| "Domain must be a valid IDNA DNS name without wildcards or URL syntax")?;
    let domain = ascii
        .strip_suffix('.')
        .unwrap_or(&ascii)
        .to_ascii_lowercase();
    if domain.is_empty() || domain.parse::<IpAddr>().is_ok() {
        return Err("Domain must be a nonempty DNS name, not a literal IP address");
    }
    Ok(domain)
}

fn diagnostic(
    diagnostics: &mut Vec<PolicyDiagnostic>,
    rule_id: Option<&str>,
    field: impl Into<String>,
    message: impl Into<String>,
) {
    if diagnostics.len() < MAX_DIAGNOSTICS {
        diagnostics.push(PolicyDiagnostic {
            rule_id: rule_id.map(str::to_owned),
            field: field.into(),
            message: message.into(),
        });
    }
}

fn empty_decision(
    action: RouteDecisionAction,
    rule_id: Option<&str>,
    reason: &str,
) -> RouteDecision {
    RouteDecision {
        action,
        rule_id: rule_id.map(str::to_owned),
        path: vec![],
        exit_node_id: None,
        reason: reason.to_owned(),
    }
}

fn valid_preview(decision: RouteDecision) -> PreviewResult {
    PreviewResult {
        valid: true,
        diagnostics: vec![],
        decision,
    }
}

fn invalid_preview(rule_id: Option<&str>, field: &str, reason: &str) -> PreviewResult {
    let mut diagnostics = vec![];
    diagnostic(&mut diagnostics, rule_id, field, reason);
    PreviewResult {
        valid: false,
        diagnostics,
        decision: empty_decision(RouteDecisionAction::Invalid, rule_id, reason),
    }
}

/// Count serialized bytes without allocating an additional policy-sized buffer.
fn policy_fits(policy: &RoutingPolicy) -> bool {
    struct BoundedWriter(usize);
    impl io::Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_POLICY_BYTES - self.0 {
                return Err(io::Error::other("policy size limit exceeded"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(BoundedWriter(0), policy).is_ok()
}

#[cfg(test)]
mod tests;
