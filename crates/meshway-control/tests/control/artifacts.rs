//! Rule-set artifacts of the held netmap: fetched by digest from the real
//! Server, cached read-only beside `netmap.json`, reloaded offline and removed
//! once no held netmap references them.
use crate::support::{Owner, Server, request};
use meshway_configuration::NetworkConfigurationInput;
use meshway_protocol::{ConfigurationRequest, WireSecret};
use meshway_test_server::client::{
    ApplyConfigurationPath, GetConfigurationSourcePath, GetConfigurationSourceQuery,
};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

async fn configure(server: &Server, owner: &Owner, cidr: &str) {
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
    let mut input: NetworkConfigurationInput = source.configuration.clone();
    input.rule_sets.insert(
        "corp".into(),
        serde_json::from_value(serde_json::json!({"type": "inline", "ipCIDR": [cidr]})).unwrap(),
    );
    input.routing.egress.rules = serde_json::from_value(serde_json::json!([
        {"match": {"ruleSets": ["corp"]}, "action": {"type": "direct"}}
    ]))
    .unwrap();
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

async fn fetch(control: &mut meshway_control::Control) -> bool {
    let call = control.begin_artifacts().expect("a due artifact");
    let reply = call.send().await;
    control.finish_artifacts(reply).await
}

#[tokio::test]
async fn artifacts_are_fetched_once_cached_read_only_reloaded_offline_and_collected() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("artifacts");
    let mut control = server
        .control(&owner, &server.origin, "artifacts", BTreeMap::new())
        .await;
    configure(&server, &owner, "9.9.9.0/24").await;
    crate::support::sync(&mut control).await.unwrap();
    let netmap = control.netmap().unwrap();
    let first = netmap.content.netmap.projection.rule_sets["corp"].digest;
    // Content is published at once, without the artifact.
    assert!(netmap.artifacts.is_empty());
    assert_eq!(control.missing_artifacts(), vec![first]);
    assert_eq!(control.status().missing_artifacts, vec![first]);

    assert!(fetch(&mut control).await);
    assert!(control.missing_artifacts().is_empty());
    assert!(control.begin_artifacts().is_none());
    let netmap = control.netmap().unwrap();
    assert_eq!(netmap.artifacts[&first].digest(), first);
    let file = path.join("artifacts").join(first.file_name());
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
        0o400
    );
    // The engine computes desired state over the loaded set.
    let desired = meshway_engine::Desired::new(&netmap.content, &netmap.artifacts).unwrap();
    assert!(desired.missing().is_empty());
    control.shutdown().await.unwrap();

    // Restart without any Server contact: the cache is verified and loaded.
    let mut control =
        meshway_control::Control::open(&path, &server.origin, meshway_test_server::http()).unwrap();
    let netmap = control.netmap().expect("offline authority");
    assert!(netmap.artifacts.contains_key(&first));
    assert!(control.missing_artifacts().is_empty());

    // New content referencing another set: the old file is removed as soon as
    // the new netmap is held; the new one is fetched.
    control.authenticate().await.unwrap();
    configure(&server, &owner, "1.1.1.0/24").await;
    crate::support::sync(&mut control).await.unwrap();
    let second = control
        .netmap()
        .unwrap()
        .content
        .netmap
        .projection
        .rule_sets["corp"]
        .digest;
    assert_ne!(second, first);
    assert_eq!(control.missing_artifacts(), vec![second]);
    assert!(!file.exists());
    assert!(fetch(&mut control).await);
    assert!(path.join("artifacts").join(second.file_name()).exists());
    control.shutdown().await.unwrap();
    server.close().await;
}
