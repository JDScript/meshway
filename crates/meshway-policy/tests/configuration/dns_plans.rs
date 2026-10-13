//! Saving validates the DNS plan every Node would execute, with the same
//! function its netmap uses: pure IP rules resolve locally whatever exits
//! they use, name rules resolve through their one exit, and name rules
//! through two exits are rejected at the rule.
use super::*;
use meshway_policy::compile::{DnsResolution, dns_resolution};
use std::sync::Arc;

fn exit_rule(matcher: serde_json::Value, exit: u8) -> serde_json::Value {
    serde_json::json!({"match": matcher,
        "action": {"type": "exit", "nodes": [format!("node:{}", id(exit))], "via": []}})
}

/// Nodes 2 and 3 are exits; every node may reach the Internet. `cn` is an
/// IP-only set (as `geoip:cn`), `names` one with names.
fn network(rules: Vec<serde_json::Value>) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    for exit in [2, 3] {
        c.nodes.get_mut(&id(exit)).unwrap().exit_enabled = true;
    }
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    for (key, ip_cidr, domain) in [
        ("cn", vec!["1.2.4.0/22".to_owned()], vec![]),
        ("names", vec![], vec!["telegram.org".to_owned()]),
    ] {
        c.rule_sets.insert(
            key.into(),
            RuleSet::Inline {
                ip_cidr,
                domain_suffix: vec![],
                domain,
                domain_subdomain: vec![],
            },
        );
    }
    c.routing =
        serde_json::from_value(serde_json::json!({ "egress": { "rules": rules } })).unwrap();
    c
}

fn sets(configuration: &NetworkConfiguration) -> meshway_policy::compile::RuleSets {
    configuration
        .rule_sets
        .iter()
        .filter_map(|(key, set)| match set {
            RuleSet::Inline {
                ip_cidr,
                domain_suffix,
                domain,
                domain_subdomain,
            } => meshway_rules::RuleSet::from_inline(&meshway_rules::inline::Inline {
                domain,
                domain_suffix,
                domain_subdomain,
                ip_cidr,
            })
            .ok()
            .map(|set| (key.clone(), Arc::new(set))),
            RuleSet::Remote { .. } => None,
        })
        .collect()
}

/// Saved: node 1's resolution. Rejected: the issue paths and the nodes.
fn judge(rules: Vec<serde_json::Value>) -> Result<DnsResolution, Vec<(String, String)>> {
    match normalize_with_dependencies(network(rules), &context()) {
        Ok(normalized) => {
            let configuration = normalized.configuration;
            Ok(dns_resolution(&configuration, &sets(&configuration), "mesh", id(1)).unwrap())
        }
        Err(ConfigurationError::Invalid(issues)) => Err(issues
            .into_iter()
            .filter(|issue| issue.code == IssueCode::DnsExitPlanRequired)
            .map(|issue| (issue.path, issue.params["node"].clone()))
            .collect()),
        Err(other) => panic!("{other:?}"),
    }
}

#[test]
fn ip_rules_resolve_locally_and_name_rules_through_their_one_exit() {
    let ip = |cidr: &str| serde_json::json!({"ipCIDR": [cidr]});
    let set = |key: &str| serde_json::json!({"ruleSets": [key]});
    let name = |suffix: &str| serde_json::json!({"domainSuffix": [suffix]});
    // Pure IP, one exit.
    assert_eq!(
        judge(vec![exit_rule(ip("128.238.0.0/16"), 2)]),
        Ok(DnsResolution::Local)
    );
    // Pure IP, two exits; an IP-only set counts as IP.
    assert_eq!(
        judge(vec![
            exit_rule(ip("128.238.0.0/16"), 2),
            exit_rule(set("cn"), 3)
        ]),
        Ok(DnsResolution::Local)
    );
    // A name rule decides the resolver; IP rules through another exit do not.
    for names in [name("example.com"), set("names")] {
        assert_eq!(
            judge(vec![
                exit_rule(ip("128.238.0.0/16"), 2),
                exit_rule(names, 3)
            ]),
            Ok(DnsResolution::Exit {
                exit: id(3),
                via: vec![]
            })
        );
    }
    // Name rules through two exits: rejected at the second, for node 1
    // only (an exit executes its own rule as direct).
    assert_eq!(
        judge(vec![
            exit_rule(name("example.com"), 2),
            exit_rule(set("names"), 3)
        ]),
        Err(vec![(
            "/routing/egress/rules/1".to_owned(),
            id(1).to_string()
        )])
    );
}

/// A kept resolver an exit profile would capture is reached directly; a
/// warning names it for each Node whose Access does not cover its address.
#[test]
fn a_kept_resolver_outside_access_is_a_warning() {
    let warnings = |grants: &[&str]| {
        let mut c = network(vec![]);
        c.access[0].dst = grants.iter().map(|g| (*g).into()).collect();
        let mut dns = serde_json::to_value(&c.dns).unwrap();
        dns["servers"] = serde_json::json!(
            {"office": {"type": "udp", "server": "10.20.0.53", "useWithExit": true}});
        dns["routes"] = serde_json::json!({"corp.example": ["office"]});
        c.dns = serde_json::from_value(dns).unwrap();
        let global = format!("global:{}", id(2));
        c.routing = serde_json::from_value(serde_json::json!({"egress": {"assignments": [
            {"target": [format!("node:{}", id(1))], "allowed": [global], "default": global, "locked": false}
        ]}}))
        .unwrap();
        normalize_with_dependencies(c, &context())
            .unwrap()
            .warnings
            .into_iter()
            .filter(|w| w.code == IssueCode::ResolverOutsideAccess)
            .map(|w| (w.path, w.params["node"].clone(), w.params["server"].clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        warnings(&["autogroup:internet"]),
        [(
            "/dns/servers/office".to_owned(),
            id(1).to_string(),
            "office".to_owned()
        )]
    );
    assert!(warnings(&["autogroup:internet", "10.20.0.0/16"]).is_empty());
}
