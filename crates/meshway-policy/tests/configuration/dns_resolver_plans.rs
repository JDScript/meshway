//! Saving refuses DNS settings a Node could not install
//! (`dns_resolver_plan_incomplete`), judged for every Node and profile with
//! the function its DNS owner installs by: a search domain no resolver of
//! the plan answers, a loopback or link-local resolver, a resolver at a Mesh
//! address or behind an exit without `useWithExit`.
use super::*;

/// Every node may reach the Internet; node 2 is an exit.
fn network(dns: serde_json::Value) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    let mut value = serde_json::to_value(&c.dns).unwrap();
    for (key, field) in dns.as_object().unwrap() {
        value[key] = field.clone();
    }
    c.dns = serde_json::from_value(value).unwrap();
    c
}

/// The issues of `code` as (path, node, profile, reason).
fn gaps(c: NetworkConfigurationInput) -> Vec<(String, String, String, String)> {
    match normalize_with_dependencies(c, &context()) {
        Ok(_) => Vec::new(),
        Err(ConfigurationError::Invalid(issues)) => {
            let found: Vec<_> = issues
                .iter()
                .filter(|issue| issue.code == IssueCode::DnsResolverPlanIncomplete)
                .map(|issue| {
                    (
                        issue.path.clone(),
                        issue.params["node"].clone(),
                        issue.params["profile"].clone(),
                        issue.params["reason"].clone(),
                    )
                })
                .collect();
            assert!(!found.is_empty(), "{issues:?}");
            found
        }
        Err(other) => panic!("{other:?}"),
    }
}

fn every_node(path: &str, reason: &str) -> Vec<(String, String, String, String)> {
    (1..=3)
        .map(|n| {
            (
                path.to_owned(),
                id(n).to_string(),
                "mesh".to_owned(),
                reason.to_owned(),
            )
        })
        .collect()
}

#[test]
fn a_search_domain_without_a_resolver_is_refused_for_every_node() {
    // M1: MagicDNS and one search domain, no resolver at all.
    assert_eq!(
        gaps(network(serde_json::json!({
            "magicDNS": true, "servers": {}, "default": [], "routes": {},
            "searchDomains": ["churn1.m1.test"]
        }))),
        every_node("/dns/searchDomains/0", "search_domain_without_resolver")
    );
    // The same without MagicDNS.
    assert_eq!(
        gaps(network(serde_json::json!({
            "magicDNS": false, "searchDomains": ["Corp.Example."]
        }))),
        every_node("/dns/searchDomains/0", "search_domain_without_resolver")
    );
    let office = serde_json::json!({"office": {"type": "udp", "server": "10.20.0.53"}});
    // Under the MagicDNS domain, under a split route, or with a fallback:
    // a resolver of the plan answers it.
    for dns in [
        serde_json::json!({"magicDNS": true, "searchDomains": ["mesh.example", "x.mesh.example"]}),
        serde_json::json!({"servers": office, "routes": {"corp.example": ["office"]},
            "searchDomains": ["corp.example", "eu.corp.example"]}),
        serde_json::json!({"servers": office, "default": ["office"], "overrideLocal": true,
            "searchDomains": ["elsewhere.example"]}),
    ] {
        assert_eq!(gaps(network(dns.clone())), vec![], "{dns}");
    }
    // A label suffix only: `evilcorp.example` is not under `corp.example`.
    assert_eq!(
        gaps(network(serde_json::json!({"servers": office,
            "routes": {"corp.example": ["office"]}, "searchDomains": ["evilcorp.example"]}))),
        every_node("/dns/searchDomains/0", "search_domain_without_resolver")
    );
}

#[test]
fn a_fallback_through_an_exit_answers_a_search_domain() {
    // Node 1 may only execute the global exit profile: its names resolve
    // through the exit, which covers the search domain. Nodes 2 and 3
    // execute `mesh` and are refused.
    let mut c = network(serde_json::json!({"searchDomains": ["corp.example"]}));
    let global = format!("global:{}", id(2));
    c.routing = serde_json::from_value(serde_json::json!({"egress": {"assignments": [
        {"target": [format!("node:{}", id(1))], "allowed": [global], "default": global, "locked": false}
    ]}}))
    .unwrap();
    assert_eq!(
        gaps(c),
        every_node("/dns/searchDomains/0", "search_domain_without_resolver")[1..].to_vec()
    );
}

#[test]
fn resolvers_a_node_cannot_query_are_refused() {
    let server = |address: &str, keep: bool| serde_json::json!({"r": {"type": "udp", "server": address, "useWithExit": keep}});
    // Unreferenced: a Node still checks every resolver it may query.
    for address in ["127.0.0.53", "::1", "fe80::1"] {
        assert_eq!(
            gaps(network(
                serde_json::json!({"servers": server(address, false)})
            )),
            every_node("/dns/servers/r", "local_resolver"),
            "{address}"
        );
    }
    // A Mesh address: a pool address or a node's own.
    for address in ["100.80.9.9", "100.80.0.3"] {
        assert_eq!(
            gaps(network(
                serde_json::json!({"servers": server(address, false)})
            )),
            every_node("/dns/servers/r", "routed_resolver"),
            "{address}"
        );
    }
    // An IP rule sends the resolver through the exit; node 2 executes its
    // own rule as direct. `useWithExit` makes it the source's direct
    // exception.
    let exit_rule = serde_json::json!({"egress": {"rules": [{"match": {"ipCIDR": ["8.8.8.0/24"]},
        "action": {"type": "exit", "nodes": [format!("node:{}", id(2))], "via": []}}]}});
    let mut c = network(serde_json::json!({"servers": server("8.8.8.8", false)}));
    c.routing = serde_json::from_value(exit_rule.clone()).unwrap();
    let mut expected = every_node("/dns/servers/r", "routed_resolver");
    expected.remove(1);
    assert_eq!(gaps(c), expected);
    let mut c = network(serde_json::json!({"servers": server("8.8.8.8", true)}));
    c.routing = serde_json::from_value(exit_rule).unwrap();
    assert_eq!(gaps(c), vec![]);
    assert_eq!(
        gaps(network(
            serde_json::json!({"servers": server("1.1.1.1", false)})
        )),
        vec![]
    );
}

#[test]
fn more_domains_than_a_node_installs_are_refused() {
    let office = serde_json::json!({"office": {"type": "udp", "server": "10.20.0.53"}});
    let routes = |count: usize| {
        (0..count)
            .map(|i| (format!("s{i}.example"), serde_json::json!(["office"])))
            .collect::<serde_json::Map<_, _>>()
    };
    // 255 routes and the MagicDNS domain: 256.
    assert_eq!(
        gaps(network(
            serde_json::json!({"servers": office, "routes": routes(255)})
        )),
        vec![]
    );
    assert_eq!(
        gaps(network(
            serde_json::json!({"servers": office, "routes": routes(256)})
        )),
        every_node("/dns", "domain_limit")
    );
}

/// Before the first Node enrolls: refused now rather than at its enrollment.
#[test]
fn a_network_without_nodes_is_judged_as_its_first_node() {
    let mut c = input();
    let mut dns = serde_json::to_value(&c.dns).unwrap();
    dns["searchDomains"] = serde_json::json!(["churn1.m1.test"]);
    dns["servers"] = serde_json::json!({"stub": {"type": "udp", "server": "127.0.0.1"}});
    c.dns = serde_json::from_value(dns).unwrap();
    let issues = issues(normalize_with_dependencies(c, &context()));
    let found: Vec<_> = issues
        .iter()
        .filter(|issue| issue.code == IssueCode::DnsResolverPlanIncomplete)
        .map(|issue| {
            assert!(!issue.params.contains_key("node"));
            assert_eq!(issue.params["profile"], "mesh");
            (issue.path.as_str(), issue.params["reason"].as_str())
        })
        .collect();
    assert_eq!(
        found,
        [
            ("/dns/servers/stub", "local_resolver"),
            ("/dns/searchDomains/0", "search_domain_without_resolver")
        ]
    );
}
