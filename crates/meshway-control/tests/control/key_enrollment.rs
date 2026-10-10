use super::{
    faults::{Fault, Proxy},
    support::{Server, request},
};
use meshway_control::Control;
use meshway_control_client::{
    ApplyConfigurationPath, CreateEnrollmentKeyPath, GetConfigurationSourcePath,
    GetConfigurationSourceQuery,
};
use meshway_protocol::{
    ConfigurationRequest, CreateEnrollmentKeyRequest, WireSecret, enrollment::InitialNodeSettings,
};

#[tokio::test]
async fn access_key_enrollment_recovers_lost_commit_without_browser_credentials() {
    let server = Server::start().await;
    let owner = server.owner().await;
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
    let mut configuration =
        meshway_configuration::NetworkConfigurationInput::from(&source.configuration);
    configuration
        .tags_owners
        .insert("tag:service".into(), vec![]);
    server
        .client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: owner.network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request(),
                source_jsonc: WireSecret::new(serde_json::to_string(&configuration).unwrap()),
            },
            owner.token.expose(),
        )
        .await
        .unwrap();
    let key = server
        .client
        .create_enrollment_key(
            &CreateEnrollmentKeyPath {
                network_id: owner.network,
            },
            &CreateEnrollmentKeyRequest {
                request_id: request(),
                tags: vec!["tag:service".into()],
                initial_settings: InitialNodeSettings {
                    hostname: None,
                    listeners: Default::default(),
                },
                expires_at_ms: (chrono::Utc::now() + chrono::Duration::minutes(5))
                    .timestamp_millis()
                    .try_into()
                    .unwrap(),
            },
            owner.token.expose(),
        )
        .await
        .unwrap()
        .secret
        .unwrap();
    let proxy = Proxy::start(&server.origin).await;
    let path = server.path("key-enrollment");
    let mut control = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    proxy.fault("/enrollments", Fault::Lost);
    assert!(control.enroll_with_key("key-node", key).await.is_err());
    assert!(control.key_enrollment_pending());
    let intent = control.status().enrollment_request_id.unwrap();
    assert!(control.begin_enrollment("key-node").await.is_err());
    control.shutdown().await.unwrap();
    let mut control = Control::open(
        &path,
        &proxy.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert_eq!(control.status().enrollment_request_id, Some(intent));
    let binding = control.resume_key_enrollment().await.unwrap();
    assert_eq!(binding.network_id, owner.network);
    assert!(!control.key_enrollment_pending());
    control.authenticate().await.unwrap();
    control.acquire_instance(Default::default()).await.unwrap();
    crate::support::sync(&mut control).await.unwrap();
    assert!(matches!(
        control
            .netmap_snapshot()
            .unwrap()
            .netmap()
            .unwrap()
            .netmap
            .self_
            .identity,
        meshway_configuration::NodeIdentity::Tag { .. }
    ));
    control.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}
