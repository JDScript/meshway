use super::*;

pub(super) fn pools() -> AddressPools {
    AddressPools {
        ipv4: vec!["100.80.0.0/16".into()],
        ipv6: vec!["fd42:7a65:8800::/48".into()],
    }
}
pub(super) fn initial() -> NetworkConfiguration {
    new_network(pools(), "Mesh.Example.".into()).unwrap()
}
pub(super) fn input() -> NetworkConfigurationInput {
    parse_jsonc(canonical_json(&initial()).unwrap().expose()).unwrap()
}
pub(super) fn id(n: u8) -> NodeId {
    format!("00000000-0000-0000-0000-{n:012}").parse().unwrap()
}
pub(super) fn user_identity() -> NodeIdentity {
    NodeIdentity::User {
        user: "alice@example.com".into(),
    }
}
pub(super) fn context() -> ValidationContext<'static> {
    ValidationContext {
        members: BTreeMap::from([(
            "alice@example.com".into(),
            UserBinding {
                user_id: "00000000-0000-0000-0000-000000000010".parse().unwrap(),
                membership_id: "00000000-0000-0000-0000-000000000020".parse().unwrap(),
            },
        )]),
        nodes: [id(1), id(2), id(3)].into_iter().collect(),
        approved_identities: [id(1), id(2), id(3)]
            .into_iter()
            .map(|id| (id, user_identity()))
            .collect(),
        ..Default::default()
    }
}
pub(super) fn node(n: u8) -> NodeSettingsInput {
    serde_json::from_str(&format!(r#"{{"hostname":"node-{n}","identity":{{"type":"user","user":"alice@example.com"}},"addresses":{{"ipv4":"100.80.0.{n}","ipv6":null}}}}"#)).unwrap()
}
pub(super) fn with_nodes() -> NetworkConfigurationInput {
    let mut c = input();
    c.nodes = [(id(1), node(1)), (id(2), node(2)), (id(3), node(3))]
        .into_iter()
        .collect();
    c
}

pub(super) fn egress_input(grants: &[&str], destination: &str) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = grants
        .iter()
        .map(|grant| AccessRule {
            remark: String::new(),
            src: vec![format!("node:{}", id(1))],
            dst: vec![(*grant).into()],
            ip: vec!["*".into()],
            bidirectional: false,
        })
        .collect();
    c.routing = serde_json::from_value(serde_json::json!({
        "egress": {"profiles":{"work":{"rules":[{"match":{"ipCIDR":[destination]},
            "action":{"type":"exit","nodes":[format!("node:{}",id(2))],"via":[]}}],
            "fallback":{"type":"unmanaged"}}},
            "assignments":[{"target":[format!("node:{}",id(1))],"allowed":["work"],"default":"work","locked":false}]}
    })).unwrap();
    c
}

pub(super) fn invalid(
    value: Result<(NetworkConfiguration, BTreeMap<String, UserBinding>), ConfigurationError>,
    text: &str,
) {
    let ConfigurationError::Invalid(issues) = value.unwrap_err() else {
        panic!("expected semantic error");
    };
    assert!(
        issues.iter().any(|i| i.message.contains(text)),
        "{issues:?}"
    );
}

pub(super) fn issues(
    value: Result<NormalizedConfiguration, ConfigurationError>,
) -> Vec<ConfigurationIssue> {
    match value {
        Err(ConfigurationError::Invalid(issues)) => issues,
        Err(other) => panic!("expected validation issues, got {other:?}"),
        Ok(_) => panic!("expected validation issues, got a normalized configuration"),
    }
}

pub(super) fn domain_rules(access: &[&str], rules: serde_json::Value) -> NetworkConfigurationInput {
    let mut c = with_nodes();
    c.nodes.get_mut(&id(2)).unwrap().exit_enabled = true;
    c.access = vec![AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: access.iter().map(|d| (*d).into()).collect(),
        ip: vec!["*".into()],
        bidirectional: false,
    }];
    c.rule_sets.insert(
        "names".into(),
        RuleSet::Inline {
            ip_cidr: vec![],
            domain_suffix: vec![],
            domain: vec!["Telegram.org.".into()],
        },
    );
    c.routing =
        serde_json::from_value(serde_json::json!({ "egress": { "rules": rules } })).unwrap();
    c
}
