use super::*;
use meshway_configuration::execution::DialEndpointSource;

fn locals(node: &mut Node, addresses: &[&str]) {
    node.report
        .state
        .sockets
        .get_mut("socket")
        .unwrap()
        .local_endpoints = Some(
        addresses
            .iter()
            .map(|address| LocalEndpoint {
                address: address.parse().unwrap(),
                source: LocalEndpointSource::Local,
            })
            .collect(),
    );
}

async fn target(app: &App, node: &Node) -> CompiledNode {
    app.execution()
        .configuration(
            &node.token,
            NodeConfigurationRequest {
                instance: node.instance.clone(),
                base: None,
            },
        )
        .await
        .unwrap()
        .full
        .unwrap()
}

#[tokio::test]
async fn local_candidates_publish_union_ip_change_and_withdrawal_without_changing_permissions() {
    let (_dir, app, _network, a, mut b, _) = setup(true).await;
    let before = target(&app, &a).await;
    locals(&mut b, &["10.1.2.3:23456", "10.1.2.4:23456"]);
    b.publish(&app).await;
    let first = target(&app, &a).await;
    let key = format!("{}/udp", b.id);
    let endpoints = &first.configuration.dialers[&key].endpoints;
    assert_eq!(
        &endpoints[..endpoints.len() - 2],
        before.configuration.dialers[&key].endpoints
    );
    assert_eq!(
        endpoints[endpoints.len() - 2].source,
        DialEndpointSource::Local
    );
    assert_eq!(endpoints.last().unwrap().host, "10.1.2.4");
    assert_ne!(
        first.configuration.publication,
        before.configuration.publication
    );
    assert_eq!(first.configuration.permits, before.configuration.permits);
    assert_eq!(first.configuration.peers, before.configuration.peers);
    assert_eq!(
        first.digest,
        meshway_configuration::digest(&first.configuration).unwrap()
    );
    locals(&mut b, &["10.1.2.99:23456"]);
    b.publish(&app).await;
    let changed = target(&app, &a).await;
    assert_ne!(
        changed.configuration.publication,
        first.configuration.publication
    );
    assert_eq!(
        changed.configuration.dialers[&key]
            .endpoints
            .last()
            .unwrap()
            .host,
        "10.1.2.99"
    );
    assert!(
        !changed.configuration.dialers[&key]
            .endpoints
            .iter()
            .any(|endpoint| endpoint.host == "10.1.2.3")
    );
    locals(&mut b, &[]);
    b.publish(&app).await;
    let empty = target(&app, &a).await;
    assert_eq!(
        empty.configuration.dialers[&key].endpoints,
        before.configuration.dialers[&key].endpoints
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_candidates_reject_budget_mesh_link_local_wrong_port_and_duplicate_reports_do_not_republish()
 {
    let (_dir, app, _network, a, mut b, _) = setup(true).await;
    for addresses in [
        vec!["169.254.1.1:23456"],
        vec!["127.0.0.1:23456"],
        vec!["100.64.0.1:23456"],
        vec!["10.1.2.3:1234"],
        vec!["10.1.2.3:23456"; 9],
    ] {
        locals(&mut b, &addresses);
        let mut report = b.report.clone();
        report.sequence = report.sequence.checked_next().unwrap();
        assert!(
            app.node_state_service()
                .unwrap()
                .accept_report(b.token.clone(), report)
                .await
                .is_err()
        );
    }
    locals(&mut b, &["10.1.2.3:23456"]);
    b.publish(&app).await;
    let first = target(&app, &a).await;
    let accepted = app
        .node_state_service()
        .unwrap()
        .accept_report(b.token.clone(), b.report.clone())
        .await
        .unwrap();
    assert!(accepted.duplicate);
    assert_eq!(
        target(&app, &a).await.configuration.publication,
        first.configuration.publication
    );
    b.publish(&app).await;
    assert_eq!(
        target(&app, &a).await.configuration.publication,
        first.configuration.publication
    );
    let addresses: Vec<_> = (1..=8).map(|last| format!("10.1.3.{last}:23456")).collect();
    locals(
        &mut b,
        &addresses.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    b.publish(&app).await;
    let bounded = target(&app, &a).await;
    assert_eq!(
        bounded.configuration.dialers[&format!("{}/udp", b.id)]
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.source == DialEndpointSource::Local)
            .count(),
        8
    );
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn local_candidates_do_not_create_a_missing_authorized_edge() {
    let (_dir, app, _network, a, mut b, _) = setup(false).await;
    locals(&mut b, &["10.1.2.3:23456"]);
    b.publish(&app).await;
    assert!(target(&app, &a).await.configuration.dialers.is_empty());
    app.shutdown().await.unwrap();
}
