use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server, request},
    system,
};
use meshway_configuration::{
    NetworkConfigurationInput,
    execution::{BusinessProtocol, BusinessTarget},
};
use meshway_control::{Control, Error};
use meshway_control_client::{
    ApplyConfigurationPath, GetConfigurationSourcePath, GetConfigurationSourceQuery,
};
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_protocol::{
    ConfigurationRequest, DemandRequest, InstanceRequest, RenewDemandRequest, WireSecret,
};
use std::{collections::BTreeMap, time::Duration};

pub(super) async fn control(server: &Server, owner: &Owner, origin: &str, name: &str) -> Control {
    server
        .control(
            owner,
            origin,
            name,
            BTreeMap::from([(
                "transport:native_udp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Udp],
                },
            )]),
        )
        .await
}

pub(super) async fn listeners(server: &Server, owner: &Owner) {
    let source = server
        .client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: owner.network,
            },
            &GetConfigurationSourceQuery::default(),
            owner.token.expose(),
        )
        .await
        .unwrap();
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    for settings in input.nodes.values_mut() {
        settings.listeners =
            serde_json::from_str(r#"{"udp":{"type":"native_udp","listenPort":23456}}"#).unwrap();
    }
    server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: owner.network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
}

pub(super) fn demand(source: &Control, target: &Control, port: u16) -> DemandRequest {
    let lease = source.status().instance.unwrap();
    DemandRequest {
        instance: InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        },
        request_id: request(),
        target: BusinessTarget::Mesh {
            node: target.status().binding.unwrap().node_id,
        },
        protocol: BusinessProtocol::Udp,
        port: Some(port.try_into().unwrap()),
        profile: "mesh".into(),
        duration_seconds: 30,
    }
}

#[tokio::test]
async fn atomic_refresh_survives_renewal_burst_after_commit_before_delivery_without_half_targets() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut a = control(&server, &owner, &server.origin, "refresh-a").await;
    let mut b = control(&server, &owner, &proxy.origin, "refresh-b").await;
    listeners(&server, &owner).await;
    let mut requests = Vec::new();
    for port in [53, 443, 5353, 8443] {
        let request = demand(&a, &b, port);
        a.create_demand(&request).await.unwrap();
        requests.push(request);
    }
    b.refresh_configuration().await.unwrap();
    b.acknowledge_saved().await.unwrap();
    let original = b.execution_snapshot().unwrap();
    let original_status = b.snapshot();
    let original_instance = original_status.status().instance.unwrap();
    assert_eq!(original.configuration().unwrap().permits.len(), 4);
    let deadline = a.status().instance.unwrap().expires_at_ms;
    a.renew_demand(&RenewDemandRequest {
        instance: requests[0].instance.clone(),
        request_id: requests[0].request_id,
        expires_at_ms: deadline - 20_000,
    })
    .await
    .unwrap();
    proxy.fault("/configuration/refresh", Fault::Hold);
    let granted;
    {
        let refresh = b.refresh_configuration();
        tokio::pin!(refresh);
        tokio::select! {
            result = &mut refresh => panic!("refresh delivered before release: {result:?}"),
            result = tokio::time::timeout(Duration::from_secs(2), proxy.committed()) => result.unwrap(),
        }
        for request in &requests[1..] {
            a.renew_demand(&RenewDemandRequest {
                instance: request.instance.clone(),
                request_id: request.request_id,
                expires_at_ms: deadline,
            })
            .await
            .unwrap();
        }
        // The owner has observed no intermediate target. A Server publication
        // alone does not mutate the old local snapshot or its absolute lease.
        original.ensure_current().unwrap();
        proxy.release();
        granted = refresh.await.unwrap().unwrap();
    }
    let held = b.execution_snapshot().unwrap();
    let refreshing_status = original_status.status();
    assert!(refreshing_status.authenticated);
    let retained_instance = refreshing_status.instance.unwrap();
    assert_eq!(retained_instance.instance_id, original_instance.instance_id);
    assert_eq!(retained_instance.generation, original_instance.generation);
    assert!(retained_instance.execution_expires_at_ms.is_none());
    assert!(
        original.ensure_current().is_err(),
        "status cannot resurrect output authority"
    );
    assert_eq!(held.reference(), granted);
    held.continuation_from(&original).unwrap();
    assert_eq!(
        held.configuration().unwrap().permits[&requests[0].request_id].expires_at_ms,
        deadline - 20_000
    );
    assert!(
        held.configuration().unwrap().permits[&requests[1].request_id].expires_at_ms < deadline
    );
    let latest = b.refresh_configuration().await.unwrap().unwrap();
    assert_ne!(latest, granted);
    let current = b.execution_snapshot().unwrap();
    current.continuation_from(&held).unwrap();
    assert_eq!(current.configuration().unwrap().permits.len(), 4);
    for request in &requests[1..] {
        assert_eq!(
            current.configuration().unwrap().permits[&request.request_id].expires_at_ms,
            deadline
        );
    }
    assert_eq!(proxy.count("/configuration/query"), 0);
    assert_eq!(proxy.count("/instances/renew"), 0);
    assert_eq!(proxy.count("/configuration/refresh"), 3);
    // Observed but unhanded-off intermediate publications still break opaque
    // continuity. Atomic delivery must not relax that existing proof boundary.
    for port in [9000, 9001] {
        let request = demand(&a, &b, port);
        a.create_demand(&request).await.unwrap();
        b.refresh_configuration().await.unwrap();
    }
    assert!(
        b.execution_snapshot()
            .unwrap()
            .continuation_from(&current)
            .is_err()
    );
    b.shutdown().await.unwrap();
    a.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn refresh_preserves_uncertain_state_and_rejects_mismatched_lease_identity() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut node = control(&server, &owner, &proxy.origin, "refresh-fault").await;
    node.refresh_configuration().await.unwrap();
    node.acknowledge_saved().await.unwrap();
    proxy.fault("/agent-api/v1/state", Fault::Lost);
    assert!(node.report_control_state(system()).await.is_err());
    // Force a new target, then ensure the shared receive path preserves the
    // possibly accepted report's exact sequence/sample for its HTTP retry.
    listeners(&server, &owner).await;
    node.refresh_configuration().await.unwrap();
    node.retry_state_report().await.unwrap();
    let snapshot = node.execution_snapshot().unwrap();
    proxy.fault("/configuration/refresh", Fault::WrongInstance);
    assert!(matches!(
        node.refresh_configuration().await,
        Err(Error::Contract)
    ));
    assert!(snapshot.ensure_current().is_err());
    assert!(node.execution_snapshot().is_err());
    node.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn expired_udp_sibling_does_not_expire_node_handoff_for_renewed_tcp_permission() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let mut a = control(&server, &owner, &server.origin, "expiry-a").await;
    let mut b = control(&server, &owner, &server.origin, "expiry-b").await;
    listeners(&server, &owner).await;
    let mut long = demand(&a, &b, 443);
    long.protocol = BusinessProtocol::Tcp;
    long.duration_seconds = 60;
    a.create_demand(&long).await.unwrap();
    let mut short = demand(&a, &b, 53);
    short.duration_seconds = 3;
    a.create_demand(&short).await.unwrap();
    b.refresh_configuration().await.unwrap();
    let old = b.execution_snapshot().unwrap();
    let short_expiry = old.configuration().unwrap().permits[&short.request_id].expires_at_ms;
    let long_permit = old.configuration().unwrap().permits[&long.request_id].clone();
    assert!(old.expires_at_ms() > short_expiry);
    let delay = short_expiry.saturating_sub(chrono::Utc::now().timestamp_millis() as u64);
    tokio::time::sleep(Duration::from_millis(delay + 10)).await;
    old.ensure_current().unwrap();
    let deadline = a.status().instance.unwrap().expires_at_ms;
    a.renew_demand(&RenewDemandRequest {
        instance: long.instance.clone(),
        request_id: long.request_id,
        expires_at_ms: deadline,
    })
    .await
    .unwrap();
    b.refresh_configuration().await.unwrap();
    let next = b.execution_snapshot().unwrap();
    next.continuation_from(&old).unwrap();
    let config = next.configuration().unwrap();
    assert!(!config.permits.contains_key(&short.request_id));
    assert_eq!(config.permits[&long.request_id].expires_at_ms, deadline);
    assert_eq!(config.permits[&long.request_id].paths, long_permit.paths);
    assert_eq!(
        config.permits[&long.request_id].protocol,
        BusinessProtocol::Tcp
    );
    assert_eq!(config.permits[&long.request_id].port, long.port);
    b.shutdown().await.unwrap();
    a.shutdown().await.unwrap();
    server.close().await;
}
