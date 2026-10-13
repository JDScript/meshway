//! Saving rejects a configuration whose DNS plan some Node could not
//! execute, with the issue at the rule; executable plans reach the netmaps.
use super::*;
use meshway_configuration::{execution::DnsEgressUnavailable, netmap::DnsPlan};

fn exit_rule(matcher: serde_json::Value, exit: NodeId) -> serde_json::Value {
    serde_json::json!({"match": matcher,
        "action": {"type": "exit", "nodes": [format!("node:{exit}")], "via": []}})
}

fn routing(rules: Vec<serde_json::Value>) -> ConfigurationOperation {
    ConfigurationOperation::ReplaceRouting {
        routing: serde_json::from_value(serde_json::json!({"egress": {"rules": rules}})).unwrap(),
    }
}

#[tokio::test]
async fn a_dns_plan_no_node_can_execute_is_rejected_at_save() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, a, b, ..) = fixture().await;
    let owner = owner(&app).await;
    let c = super::super::node::enroll_tag_node(
        &app,
        &owner,
        network,
        &ed25519_dalek::SigningKey::from_bytes(&[9; 32]),
        "c",
        [19; 32],
    )
    .await
    .node_id;
    let exit = |node| ConfigurationOperation::SetNodeSettings {
        node_id: node,
        fields: NodeSettingsPatch {
            exit_enabled: Input::Value(true),
            ..Default::default()
        },
    };
    edit(
        &app,
        &owner,
        network,
        vec![
            exit(b),
            exit(c),
            ConfigurationOperation::ReplaceAccess {
                access: vec![AccessRule {
                    remark: String::new(),
                    src: vec!["tag:test".into()],
                    dst: vec!["autogroup:internet".into()],
                    ip: vec!["*".into()],
                    bidirectional: false,
                }],
            },
        ],
    )
    .await
    .unwrap();
    let ip = serde_json::json!({"ipCIDR": ["128.238.0.0/16"]});
    let names = |suffix: &str| serde_json::json!({"domainSuffix": [suffix]});
    let plan = |netmap: CompiledNetmap| netmap.netmap.dns.egress.clone();

    // Pure IP rules through two exits: every Node resolves locally.
    edit(
        &app,
        &owner,
        network,
        vec![routing(vec![
            exit_rule(ip.clone(), b),
            exit_rule(serde_json::json!({"ipCIDR": ["1.2.4.0/22"]}), c),
        ])],
    )
    .await
    .unwrap();
    assert_eq!(plan(load(&app, network, a).await), DnsPlan::NoExit);

    // A name rule through c decides a's resolver; the IP rule through b does
    // not. Neither exit declared the exit capability, so the service is
    // unavailable for that reason, not for want of a plan.
    edit(
        &app,
        &owner,
        network,
        vec![routing(vec![
            exit_rule(ip.clone(), b),
            exit_rule(names("example.com"), c),
        ])],
    )
    .await
    .unwrap();
    assert_eq!(
        plan(load(&app, network, a).await),
        DnsPlan::Unavailable {
            reason: DnsEgressUnavailable::ExitUnavailable
        }
    );

    // Name rules through b and c: rejected, nothing stored.
    let before = rows(&app, network).await;
    let error = edit(
        &app,
        &owner,
        network,
        vec![routing(vec![
            exit_rule(names("example.com"), b),
            exit_rule(names("example.org"), c),
        ])],
    )
    .await
    .unwrap_err();
    let crate::error::Error::InvalidConfiguration(ConfigurationError::Invalid(issues)) = error
    else {
        panic!("{error:?}");
    };
    let issue = issues
        .iter()
        .find(|issue| issue.code == IssueCode::DnsExitPlanRequired)
        .unwrap_or_else(|| panic!("{issues:?}"));
    assert_eq!(issue.path, "/routing/egress/rules/1");
    assert_eq!(issue.params["node"], a.to_string());
    assert_eq!(issue.params["profile"], "mesh");
    assert_eq!(rows(&app, network).await, before);
    app.shutdown().await.unwrap();
}
