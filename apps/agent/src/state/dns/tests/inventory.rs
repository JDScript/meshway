use super::*;
use meshway_protocol::{
    ConfigurationSnapshot, MeshNetwork, MeshNode, NodeConfiguration, compile_dns_records,
    configuration_digest,
};

#[tokio::test]
async fn forged_rootnode_alias_cannot_point_to_another_assigned_identity() {
    let (_directory, mut agent, shared, task) = fixture().await;
    let nodes = vec![
        MeshNode {
            id: agent.identity.node_id.clone().unwrap(),
            name: "demo-sha".into(),
            country: None,
            mesh_ipv4: "100.64.0.2".into(),
            mesh_ipv6: "fd00::2".into(),
        },
        MeshNode {
            id: "b".repeat(64),
            name: "RootNode".into(),
            country: None,
            mesh_ipv4: "100.64.0.3".into(),
            mesh_ipv6: "fd00::3".into(),
        },
    ];
    let mut dns = snapshot(&nodes[0].id, 1);
    dns.payload.records = compile_dns_records(&dns.payload.dns, &nodes).unwrap().0;
    rehash(&mut dns);
    let mut payload = NodeConfiguration {
        schema_version: meshway_protocol::CONFIGURATION_SCHEMA_VERSION,
        revision: 1,
        network: MeshNetwork {
            ipv4_prefix: "100.64.0.0/24".into(),
            ipv6_prefix: "fd00::/64".into(),
        },
        node: nodes[0].clone(),
        peers: vec![nodes[1].clone()],
    };
    let config = ConfigurationSnapshot {
        digest: configuration_digest(&payload).unwrap(),
        payload: payload.clone(),
    };
    crate::state::configuration::validate(&config, &agent.identity, None).unwrap();
    agent.persist_configuration(config).unwrap();
    *shared.0.write().unwrap() = Source::Snapshot(dns.clone());
    let mut runtime = sync(&agent, None);
    runtime.tick().await.unwrap();
    assert_eq!(
        query(&runtime.guard, "rootnode.meshway.test.")
            .await
            .answers
            .len(),
        1
    );
    payload.revision = 2;
    agent
        .persist_configuration(ConfigurationSnapshot {
            digest: configuration_digest(&payload).unwrap(),
            payload,
        })
        .unwrap();
    dns.payload.revision = 2;
    dns.payload.records[0].aliases = vec!["rootnode.meshway.test".into()];
    dns.payload.records[1].aliases = vec!["demo-sha.meshway.test".into()];
    rehash(&mut dns);
    // Structurally valid and uniquely named, but ownership is forged.
    dns.payload.validate().unwrap();
    *shared.0.write().unwrap() = Source::Snapshot(dns);
    assert!(runtime.tick().await.is_err());
    assert!(
        query(&runtime.guard, "rootnode.meshway.test.")
            .await
            .answers
            .is_empty()
    );
    task.abort();
}
