//! Node regions and the `auto` selector: shape, warnings, and the DNS plan
//! of every profile `auto` may resolve to.
use super::*;

/// Nodes 2 and 3 are exits; node 1 may reach the Internet. `auto` sends US
/// to `work` (exit 2) and JP to `split`; the fallback is `mesh`.
fn network(region: Option<&str>, split: serde_json::Value) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    for exit in [2, 3] {
        c.nodes.get_mut(&id(exit)).unwrap().exit_enabled = true;
    }
    c.nodes.get_mut(&id(1)).unwrap().region = region.map(str::to_owned);
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", id(1))],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    let exit =
        |n: u8| serde_json::json!({"type":"exit","nodes":[format!("node:{}", id(n))],"via":[]});
    c.routing = serde_json::from_value(serde_json::json!({"egress": {
        "profiles": {
            "work": {"rules": [], "fallback": exit(2)},
            "split": {"rules": split, "fallback": {"type": "unmanaged"}},
        },
        "assignments": [{"target": [format!("node:{}", id(1))],
            "allowed": ["auto", "work", "split", "mesh"], "default": "auto", "locked": false}],
        "auto": {"rules": [{"countries": ["US"], "profile": "work"},
            {"countries": ["JP"], "profile": "split"}], "fallback": "mesh"},
    }}))
    .unwrap();
    c
}

fn warnings(
    input: NetworkConfigurationInput,
) -> Vec<(IssueCode, String, BTreeMap<String, String>)> {
    normalize_with_dependencies(input, &context())
        .unwrap()
        .warnings
        .into_iter()
        .filter(|w| {
            matches!(
                w.code,
                IssueCode::AutoRegionUnmatched | IssueCode::AutoRegionMissing
            )
        })
        .map(|w| (w.code, w.path, w.params))
        .collect()
}

#[test]
fn a_region_is_an_uppercase_two_letter_code() {
    for bad in ["us", "USA", "U", "1A", ""] {
        let found = issues(normalize_with_dependencies(
            network(Some(bad), serde_json::json!([])),
            &context(),
        ));
        assert!(
            found.iter().any(|i| i.code == IssueCode::InvalidCountryCode
                && i.path == format!("/nodes/{}/region", id(1))
                && i.params["value"] == bad),
            "{bad}: {found:?}"
        );
    }
    let saved = normalize_with_dependencies(network(Some("US"), serde_json::json!([])), &context())
        .unwrap();
    assert_eq!(
        saved.configuration.nodes[&id(1)].region.as_deref(),
        Some("US")
    );
    // An undeclared region is omitted from the canonical document.
    let none =
        normalize_with_dependencies(network(None, serde_json::json!([])), &context()).unwrap();
    let canonical = serde_json::to_value(&none.configuration).unwrap();
    assert!(
        canonical["nodes"][id(1).to_string()]
            .get("region")
            .is_none()
    );
}

#[test]
fn an_unlisted_or_missing_region_warns_that_auto_falls_back() {
    assert_eq!(warnings(network(Some("US"), serde_json::json!([]))), []);
    let node = id(1).to_string();
    assert_eq!(
        warnings(network(Some("FR"), serde_json::json!([]))),
        [(
            IssueCode::AutoRegionUnmatched,
            format!("/nodes/{node}/region"),
            BTreeMap::from([
                ("node".to_owned(), node.clone()),
                ("region".to_owned(), "FR".to_owned())
            ])
        )]
    );
    assert_eq!(
        warnings(network(None, serde_json::json!([]))),
        [(
            IssueCode::AutoRegionMissing,
            "/routing/egress/assignments/0".to_owned(),
            BTreeMap::from([("nodes".to_owned(), node)])
        )]
    );
    // A Node that cannot select `auto` is not warned about its region.
    let mut other = network(Some("US"), serde_json::json!([]));
    other.nodes.get_mut(&id(3)).unwrap().region = Some("FR".into());
    assert_eq!(warnings(other), []);
}

#[test]
fn every_profile_auto_may_resolve_to_needs_an_executable_dns_plan_whatever_the_region() {
    let exit =
        |n: u8| serde_json::json!({"type":"exit","nodes":[format!("node:{}", id(n))],"via":[]});
    let split = serde_json::json!([
        {"match": {"domainSuffix": ["a.example"]}, "action": exit(2)},
        {"match": {"domainSuffix": ["b.example"]}, "action": exit(3)},
    ]);
    for region in [None, Some("US"), Some("JP")] {
        let found = issues(normalize_with_dependencies(
            network(region, split.clone()),
            &context(),
        ));
        assert!(
            found.iter().any(|i| i.code == IssueCode::DnsExitPlanRequired
                && i.params["profile"] == "split"),
            "{region:?}: {found:?}"
        );
    }
}
