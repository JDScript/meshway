use meshway_agent_connections::core_config;
use meshway_dataplane::{Identity, PrivateKey, TransportKind};
use meshway_protocol::NodeConnections;
use std::collections::BTreeMap;

#[test]
fn adjacency_configuration_deduplicates_targets_without_authorizing_packets_or_reverse_dials() {
    let local = Identity::generate().unwrap();
    let remote = Identity::generate().unwrap();
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let intent: NodeConnections = serde_json::from_value(serde_json::json!({
        "schema_version":1,"revision":1,"node_id":a,"public_key":local.public_key_hex(),
        "capabilities":[],"valid":true,"diagnostics":[],"listeners":[],"incoming":[],
        "peers":[{"node_id":b,"public_key":remote.public_key_hex()}],
        "outgoing":[
            {"id":"preferred","peer_node_id":b,"listener_id":"remote","transport":"native_tcp",
                "endpoint":{"host":"localhost","port":43100},"priority":0,"credential_ref":null},
            {"id":"backup-alias","peer_node_id":b,"listener_id":"remote","transport":"native_tcp",
                "endpoint":{"host":"127.0.0.1","port":43100},"priority":1,"credential_ref":null}
        ]
    }))
    .unwrap();
    let address = "127.0.0.1:43100".parse().unwrap();
    let targets = BTreeMap::from([
        ("preferred".into(), vec![address]),
        ("backup-alias".into(), vec![address]),
    ]);
    let config = core_config(
        &intent,
        PrivateKey::from_hex(&local.private_key_hex()).unwrap(),
        &targets,
    )
    .unwrap();
    assert!(config.allowed_paths.is_empty());
    assert!(config.listeners.is_empty());
    assert_eq!(config.outgoing.len(), 1);
    assert_eq!(config.outgoing[0].id.as_deref(), Some("preferred"));
    assert_eq!(config.outgoing[0].peer, b);
    assert_eq!(config.outgoing[0].transport, TransportKind::NativeTcp);
}
