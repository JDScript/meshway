use super::*;
use meshway_protocol::{
    ConfigurationSnapshot, MeshNetwork, MeshNode, NodeConfiguration, RoutingPolicy,
    configuration_digest,
};

fn disabled(id: &str, revision: u32) -> AgentDnsSnapshot {
    let mut snapshot = snapshot(id, revision);
    snapshot.payload.dns.enabled = false;
    snapshot.payload.records.clear();
    rehash(&mut snapshot);
    snapshot
}

fn assigned_configuration(agent: &mut Agent, revision: u32, v4: &str, v6: &str) {
    let payload = NodeConfiguration {
        schema_version: 1,
        revision,
        network: MeshNetwork {
            ipv4_prefix: "100.64.0.0/24".into(),
            ipv6_prefix: "fd00::/64".into(),
        },
        node: MeshNode {
            id: agent.identity.node_id.clone().unwrap(),
            name: agent.identity.name.clone(),
            country: None,
            mesh_ipv4: v4.into(),
            mesh_ipv6: v6.into(),
        },
        peers: vec![],
        routing_policy: RoutingPolicy::default(),
        policy_valid: true,
        diagnostics: vec![],
    };
    let snapshot = ConfigurationSnapshot {
        digest: configuration_digest(&payload).unwrap(),
        payload,
    };
    crate::state::configuration::validate(&snapshot, &agent.identity, None).unwrap();
    agent.persist_configuration(snapshot).unwrap();
}

#[tokio::test]
async fn disabled_and_disabled_restart_preserve_self_pin_reject_renumber_and_restore_same_addresses()
 {
    for restart in [false, true] {
        let (_directory, agent, shared, task) = fixture().await;
        assert!(!crate::state::configuration::cache_path(&agent.path).exists());
        let mut runtime = sync(&agent, None);
        runtime.tick().await.unwrap();
        let original = runtime.cache.as_ref().unwrap().self_addresses.clone();
        assert!(original.is_some());
        let id = agent.identity.node_id.as_deref().unwrap();
        *shared.0.write().unwrap() = Source::Snapshot(disabled(id, 2));
        runtime.tick().await.unwrap();
        assert_eq!(
            runtime.cache.as_ref().unwrap().self_addresses.as_ref(),
            original.as_ref()
        );
        let saved = std::fs::read(path(&agent.path)).unwrap();
        if restart {
            runtime = sync(&agent, load(&agent.path, &agent.identity).unwrap());
            assert!(
                query(&runtime.guard, "demo-sha.meshway.test.")
                    .answers
                    .is_empty()
            );
        }
        let mut changed = snapshot(id, 3);
        changed.payload.records[0].mesh_ipv4 = "100.64.0.3".into();
        changed.payload.records[0].mesh_ipv6 = "fd00::3".into();
        rehash(&mut changed);
        changed.payload.validate().unwrap();
        *shared.0.write().unwrap() = Source::Snapshot(changed);
        assert!(matches!(
            runtime.tick().await,
            Err(AgentError::Invalid(
                "DNS durable self address binding changed"
            ))
        ));
        assert!(
            query(&runtime.guard, "demo-sha.meshway.test.")
                .answers
                .is_empty()
        );
        assert_eq!(std::fs::read(path(&agent.path)).unwrap(), saved);
        assert_eq!(runtime.cache.as_ref().unwrap().snapshot.payload.revision, 2);
        *shared.0.write().unwrap() = Source::Snapshot(snapshot(id, 3));
        runtime.tick().await.unwrap();
        assert_eq!(
            query(&runtime.guard, "demo-sha.meshway.test.")
                .answers
                .len(),
            1
        );
        assert_eq!(
            load(&agent.path, &agent.identity)
                .unwrap()
                .unwrap()
                .self_addresses
                .as_ref(),
            original.as_ref()
        );
        task.abort();
    }
}

#[tokio::test]
async fn disabled_initial_unknown_pin_is_persisted_when_configuration_arrives_and_checks_config_after_restart()
 {
    let (_directory, mut agent, shared, task) = fixture().await;
    let id = agent.identity.node_id.clone().unwrap();
    *shared.0.write().unwrap() = Source::Snapshot(disabled(&id, 1));
    let mut runtime = sync(&agent, None);
    runtime.tick().await.unwrap();
    assert!(runtime.cache.as_ref().unwrap().self_addresses.is_none());
    assigned_configuration(&mut agent, 1, "100.64.0.2", "fd00::2");
    // Same DNS digest: discovering the pin must still cause private persistence.
    let digest = runtime.cache.as_ref().unwrap().snapshot.digest.clone();
    runtime.tick().await.unwrap();
    assert_eq!(runtime.cache.as_ref().unwrap().snapshot.digest, digest);
    let cache = load(&agent.path, &agent.identity).unwrap().unwrap();
    assert!(cache.self_addresses.is_some());
    let saved = std::fs::read(path(&agent.path)).unwrap();
    let mut restarted = sync(&agent, Some(cache));
    assigned_configuration(&mut agent, 2, "100.64.0.3", "fd00::3");
    *shared.0.write().unwrap() = Source::Snapshot(disabled(&id, 2));
    // Even with zero records, the separately published configuration must not
    // redefine an already pinned DNS identity address.
    assert!(matches!(
        restarted.tick().await,
        Err(AgentError::Invalid(
            "DNS durable self address binding changed"
        ))
    ));
    assert_eq!(std::fs::read(path(&agent.path)).unwrap(), saved);
    task.abort();
}

#[tokio::test]
async fn cache_v1_missing_pin_and_tampered_disabled_pin_fail_without_overwrite() {
    let (_directory, agent, shared, task) = fixture().await;
    let mut runtime = sync(&agent, None);
    runtime.tick().await.unwrap();
    *shared.0.write().unwrap() =
        Source::Snapshot(disabled(agent.identity.node_id.as_deref().unwrap(), 2));
    runtime.tick().await.unwrap();
    let mut cache = load(&agent.path, &agent.identity).unwrap().unwrap();
    #[derive(Serialize)]
    struct Legacy<'a> {
        format_version: u32,
        controller: &'a str,
        node_id: &'a str,
        credential_hash: &'a [u8],
        snapshot: &'a AgentDnsSnapshot,
    }
    for version in [1, 2] {
        let old = Legacy {
            format_version: version,
            controller: &cache.controller,
            node_id: &cache.node_id,
            credential_hash: &cache.credential_hash,
            snapshot: &cache.snapshot,
        };
        let bytes = serde_json::to_vec(&old).unwrap();
        local::atomic_write(&path(&agent.path), &bytes, true).unwrap();
        assert!(load(&agent.path, &agent.identity).is_err());
        assert_eq!(std::fs::read(path(&agent.path)).unwrap(), bytes);
    }
    cache.self_addresses.as_mut().unwrap().mesh_ipv4 = Ipv4Addr::LOCALHOST;
    let bytes = serde_json::to_vec(&cache).unwrap();
    local::atomic_write(&path(&agent.path), &bytes, true).unwrap();
    assert!(load(&agent.path, &agent.identity).is_err());
    assert_eq!(std::fs::read(path(&agent.path)).unwrap(), bytes);
    task.abort();
}
