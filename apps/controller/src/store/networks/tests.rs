use crate::{
    Error, NetworkOptions,
    store::{
        NetworkId, Store,
        test_support::{SingleNetworkFixture, directory, request},
    },
};
use meshway_protocol::*;
use rusqlite::params;

fn enrolled(store: &mut Store, network: &NetworkId, request: &EnrollAgentRequest) -> NodeSummary {
    let token = store.create_token(network, 3600).unwrap();
    store.enroll(&token.token, request).unwrap()
}

#[test]
fn networks_isolate_inventory_addresses_revision_tags_and_agent_snapshots() {
    let dir = directory();
    let mut store = Store::open(dir.path(), &NetworkOptions::default()).unwrap();
    let first_network = store.initial_network_id().unwrap();
    let first = request("one");
    let one = enrolled(&mut store, &first_network, &first);
    let first_revision = store.revision(&first_network).unwrap();
    let second_snapshot = store
        .create_network(&CreateNetworkRequest {
            name: "Second".into(),
        })
        .unwrap();
    let second_network = store.network_id(&second_snapshot.network_id).unwrap();
    assert_eq!(second_snapshot.revision, 1);
    assert!(store.nodes(&second_network).unwrap().is_empty());
    assert!(store.tags(&second_network).unwrap().tags.is_empty());
    assert!(
        store
            .connections(&second_network)
            .unwrap()
            .configuration
            .listeners
            .is_empty()
    );
    assert!(
        store
            .connection_runtimes(&second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    let second = request("one");
    let two = enrolled(&mut store, &second_network, &second);
    assert_eq!(one.mesh_ipv4, two.mesh_ipv4);
    assert!(matches!(
        store.revoke(&second_network, &one.id),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store.set_country(
            &second_network,
            &SetNodeCountryRequest {
                expected_revision: 2,
                node_id: one.id.clone(),
                country: Some("US".into())
            }
        ),
        Err(Error::NotFound)
    ));
    assert!(
        store
            .save_network_policy(
                &second_network,
                &crate::contracts::network_policy::SaveNetworkPolicyRequest {
                    expected_revision: 2,
                    policy: meshway_policy::NetworkPolicy {
                        grants: vec![meshway_policy::Grant {
                            id: "cross".into(),
                            src: vec![format!("node:{}", one.id)],
                            dst: vec!["*".into()],
                            bidirectional: false
                        }],
                        ..meshway_policy::NetworkPolicy::default_template()
                    },
                    source_jsonc: None
                }
            )
            .is_err()
    );
    assert!(
        store
            .set_node_tags(
                &second_network,
                &SetNodeTagsRequest {
                    expected_revision: 2,
                    node_id: one.id.clone(),
                    tags: vec![]
                }
            )
            .is_err()
    );
    let token = store.create_token(&second_network, 3600).unwrap();
    assert!(store.enroll(&token.token, &first).is_err());
    for _ in 0..16 {
        // Immutable IDs can be retained and used in any order. Global listing,
        // network creation and agent requests never mutate their meaning.
        assert_eq!(store.nodes(&first_network).unwrap()[0].id, one.id);
        assert_eq!(store.nodes(&second_network).unwrap()[0].id, two.id);
        assert!(
            store
                .configuration(&first.node_credential)
                .unwrap()
                .payload
                .peers
                .is_empty()
        );
        assert!(
            store
                .configuration(&second.node_credential)
                .unwrap()
                .payload
                .peers
                .is_empty()
        );
        assert_eq!(store.networks().unwrap().networks.len(), 2);
        assert_eq!(store.revision(&first_network).unwrap(), first_revision);
        assert_eq!(
            store
                .agent_dns(&first.node_credential)
                .unwrap()
                .payload
                .network_id,
            first_network.as_str()
        );
    }
    assert_eq!(
        store
            .configuration(&second.node_credential)
            .unwrap()
            .payload
            .node
            .id,
        two.id
    );
}

#[test]
fn live_discovery_and_runtime_leases_survive_network_switches() {
    let dir = directory();
    let mut fixture = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let enrollment = request("reporting");
    let network = fixture.network_id.clone();
    let node = enrolled(&mut fixture, &network, &enrollment);
    fixture
        .publish_connection_identity(
            &enrollment.node_credential,
            &PublishConnectionIdentityRequest {
                public_key: "09".to_owned() + &"00".repeat(31),
                capabilities: vec![],
            },
        )
        .unwrap();
    fixture
        .begin_connection_runtime(
            &enrollment.node_credential,
            &BeginConnectionRuntimeRequest {
                instance_id: local::credential().unwrap(),
                expected_epoch: 0,
            },
        )
        .unwrap();
    let snapshot = fixture
        .agent_connections(&enrollment.node_credential)
        .unwrap();
    fixture
        .report_connection_runtime(
            &enrollment.node_credential,
            &ReportConnectionRuntimeRequest {
                epoch: 1,
                sequence: 1,
                revision: snapshot.payload.revision,
                digest: snapshot.digest.clone(),
                state: ConnectionRuntimeState::Disabled,
                reason: ConnectionRuntimeReason::Disabled,
                listeners: vec![],
                peers: vec![],
            },
        )
        .unwrap();
    fixture
        .report_endpoint_discovery(
            &enrollment.node_credential,
            &ReportEndpointDiscoveryRequest {
                epoch: 1,
                sequence: 1,
                revision: snapshot.payload.revision,
                digest: snapshot.digest,
                state: EndpointDiscoveryState::Disabled,
                sockets: vec![],
            },
        )
        .unwrap();
    let runtime = canonical_json(&fixture.connection_runtimes().unwrap()).unwrap();
    let discovery = canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap();
    assert_eq!(
        canonical_json(&fixture.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
    let second = fixture
        .create_network(&CreateNetworkRequest {
            name: "isolated".into(),
        })
        .unwrap();
    let second_network = fixture.network_id(&second.network_id).unwrap();
    assert!(
        Store::connection_runtimes(&fixture, &second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    assert!(
        Store::endpoint_discoveries(&fixture, &second_network)
            .unwrap()
            .nodes
            .is_empty()
    );
    let other = enrolled(&mut fixture, &second_network, &request("other"));
    Store::save_tags(
        &mut fixture,
        &second_network,
        &SaveTagsRequest {
            expected_revision: 2,
            tags: vec![TagDefinition {
                tag: "tag:local".into(),
                description: "second only".into(),
            }],
        },
    )
    .unwrap();
    assert!(
        fixture
            .connection
            .execute(
                "INSERT INTO node_tags(network_id,node_id,tag) VALUES(?1,?2,'tag:local')",
                params![second_network.as_str(), node.id]
            )
            .is_err()
    );
    assert_eq!(
        Store::endpoint_discoveries(&fixture, &second_network)
            .unwrap()
            .nodes[0]
            .node_id,
        other.id
    );
    assert_eq!(
        fixture
            .get_connection_runtime_epoch(&enrollment.node_credential)
            .unwrap()
            .epoch,
        1
    );
    assert_eq!(
        canonical_json(&fixture.connection_runtimes().unwrap()).unwrap(),
        runtime
    );
    assert_eq!(
        canonical_json(&fixture.endpoint_discoveries().unwrap()).unwrap(),
        discovery
    );
}
