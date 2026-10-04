use super::{
    faults::{Fault, Proxy},
    support::{Owner, Server},
};
use meshway_configuration::node_dns_name;
use meshway_control::{Control, Error};
use meshway_control_client::{
    ApproveDeviceAuthorizationPath, GetConfigurationPath, GetNodeStatePath, RevokeNodeKeyPath,
};
use meshway_protocol::{ApproveEnrollmentRequest, EnrollmentResponse, RevokeNodeKeyRequest};

async fn enroll(
    server: &Server,
    owner: &Owner,
    control: &mut Control,
) -> (EnrollmentResponse, String) {
    let pending = control.begin_enrollment("dns-control").await.unwrap();
    server
        .client
        .approve_device_authorization(
            &ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.id,
            },
            &ApproveEnrollmentRequest {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    control.poll_enrollment().await.unwrap();
    let binding = control.redeem_enrollment().await.unwrap();
    let current = server
        .client
        .get_configuration(
            &GetConfigurationPath {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let name = node_dns_name(
        &current.configuration.nodes[&binding.node_id].hostname,
        &current.configuration.dns.domain,
    )
    .unwrap();
    (binding, name)
}

#[tokio::test]
async fn discovery_uses_generated_client_without_acquiring_reporting_or_renewing() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let path = server.path("dns-control");
    let mut control =
        Control::open(&path, &proxy.origin, reqwest::Client::builder().no_proxy()).unwrap();
    assert!(matches!(
        control.query_dns("unknown.mesh.example").await,
        Err(Error::Lifecycle)
    ));
    let (binding, name) = enroll(&server, &owner, &mut control).await;
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count("/auth/challenges"), 0);
    control.authenticate().await.unwrap();
    let snapshot = control.snapshot();
    let persisted = std::fs::read(path.join("control.json")).unwrap();
    let response = control
        .query_dns(&format!("{}.", name.to_uppercase()))
        .await
        .unwrap();
    assert_eq!(response.network_id, binding.network_id);
    assert_eq!(response.source_node_id, binding.node_id);
    let record = response.record.unwrap();
    assert_eq!(record.node_id, binding.node_id);
    assert_eq!(record.name, name);
    let current = server
        .client
        .get_configuration(
            &GetConfigurationPath {
                network_id: owner.network,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert_eq!(response.configuration_revision, current.revision);
    assert_eq!(
        record.addresses,
        current.configuration.nodes[&binding.node_id].addresses
    );
    assert!(
        control
            .query_dns("unknown.mesh.example")
            .await
            .unwrap()
            .record
            .is_none()
    );
    assert!(matches!(
        control.query_dns("*.sensitive.example").await,
        Err(Error::InvalidDnsName)
    ));
    assert_eq!(Error::InvalidDnsName.to_string(), "invalid DNS query name");
    assert!(snapshot.status().authenticated);
    assert!(control.status().instance.is_none() && control.status().cached.is_none());
    assert_eq!(std::fs::read(path.join("control.json")).unwrap(), persisted);
    let state = server
        .client
        .get_node_state(
            &GetNodeStatePath {
                network_id: binding.network_id,
                node_id: binding.node_id,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert!(state.nodes[&binding.node_id].instance_id.is_none());
    assert!(state.nodes[&binding.node_id].report.is_none());
    assert_eq!(proxy.count("/dns/query"), 2);
    assert_eq!(proxy.count("/auth/session"), 1);
    for suffix in [
        "/instances",
        "/instances/renew",
        "/configuration/query",
        "/configuration/receipts",
        "/agent-api/v1/state",
    ] {
        assert_eq!(proxy.count(suffix), 0, "{suffix}");
    }
    control.stop().await.unwrap();
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count("/dns/query"), 2);
    control.authenticate().await.unwrap();
    assert_eq!(proxy.count("/auth/session"), 1);
    // An existing lease is preserved byte-for-byte by a successful lookup.
    let lease = control.acquire_instance(Default::default()).await.unwrap();
    control.query_dns(&name).await.unwrap();
    assert_eq!(control.status().instance.unwrap(), lease);
    assert_eq!(proxy.count("/instances/renew"), 0);
    let snapshot = control.snapshot();
    server
        .client
        .revoke_node_key(
            &RevokeNodeKeyPath {
                network_id: binding.network_id,
                node_id: binding.node_id,
            },
            &RevokeNodeKeyRequest {
                key_id: binding.keys.control.id,
                expected_generation: binding.keys.control.generation,
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Server(meshway_control_client::ClientError::Http {
            status: 401,
            ..
        }))
    ));
    assert!(!snapshot.status().authenticated && snapshot.status().instance.is_none());
    assert!(!control.status().authenticated && control.status().instance.is_none());
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Lifecycle)
    ));
    assert_eq!(proxy.count("/auth/session"), 1);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn discovery_fences_foreign_bindings_names_and_malformed_generated_responses() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("dns-binding"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    let (_, name) = enroll(&server, &owner, &mut control).await;
    control.authenticate().await.unwrap();
    for fault in [
        Fault::WrongDnsNetwork,
        Fault::WrongDnsSource,
        Fault::WrongDnsName,
    ] {
        control.acquire_instance(Default::default()).await.unwrap();
        let snapshot = control.snapshot();
        proxy.fault("/dns/query", fault);
        assert!(matches!(
            control.query_dns(&name).await,
            Err(Error::Contract)
        ));
        assert!(!snapshot.status().authenticated && snapshot.status().instance.is_none());
        assert!(control.status().instance.is_none());
    }
    let snapshot = control.snapshot();
    proxy.fault("/dns/query", Fault::MissingDnsRecord);
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Server(
            meshway_control_client::ClientError::Decode { .. }
        ))
    ));
    assert!(!snapshot.status().authenticated && !control.status().authenticated);
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn discovery_rechecks_session_deadline_after_waiting_for_a_response() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let proxy = Proxy::start(&server.origin).await;
    let mut control = Control::open(
        &server.path("dns-deadline"),
        &proxy.origin,
        reqwest::Client::builder().no_proxy(),
    )
    .unwrap();
    let (_, name) = enroll(&server, &owner, &mut control).await;
    proxy.fault("/auth/session", Fault::ShortSession);
    control.authenticate().await.unwrap();
    let snapshot = control.snapshot();
    assert!(snapshot.status().authenticated);
    proxy.fault("/dns/query", Fault::Hold);
    let mut query = Box::pin(control.query_dns(&name));
    tokio::select! {
        result = &mut query => panic!("response was not held: {}", result.is_ok()),
        _ = proxy.committed() => {},
    }
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert!(!snapshot.status().authenticated);
    proxy.release();
    assert!(matches!(query.await, Err(Error::Expired)));
    assert!(matches!(
        control.query_dns(&name).await,
        Err(Error::Expired)
    ));
    assert_eq!(proxy.count("/dns/query"), 1);
    assert_eq!(proxy.count("/auth/session"), 1);
    assert!(control.status().instance.is_none());
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
