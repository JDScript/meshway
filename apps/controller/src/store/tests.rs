use meshway_protocol::*;

use super::test_support::{SingleNetworkFixture, directory, enroll, request};
use crate::{Error, NetworkOptions};

#[test]
fn dualstack_allocation_revision_and_exhaustion_are_transactional() {
    let dir = directory();
    let options = NetworkOptions {
        ipv4_prefix: Some("100.96.0.0/30".into()),
        ipv6_prefix: Some("fd12:3456:789a::/64".into()),
    };
    let mut store = SingleNetworkFixture::open(dir.path(), &options).unwrap();
    let initial = store.network_policy(&store.network_id).unwrap();
    assert_eq!(initial.revision, 1);
    assert_eq!(
        initial.policy,
        meshway_policy::NetworkPolicy::default_template()
    );
    let request = request("one");
    let token = store.create_token(3600).unwrap().token;
    assert_eq!(store.revision().unwrap(), 1);
    let first = store.enroll(&token, &request).unwrap();
    assert_eq!(first.mesh_ipv4, "100.96.0.1");
    assert_eq!(first.mesh_ipv6, "fd12:3456:789a::1");
    assert_eq!(first.configuration_revision, 2);
    assert_eq!(
        store
            .enroll(&token, &request)
            .unwrap()
            .configuration_revision,
        2
    );
    assert_eq!(
        store
            .heartbeat(&request.node_credential, "v2")
            .unwrap()
            .configuration_revision,
        2
    );
    let second_request = self::request("two");
    let second = enroll(&mut store, &second_request);
    assert_eq!(second.mesh_ipv4, "100.96.0.2");
    assert_eq!(second.mesh_ipv6, "fd12:3456:789a::2");
    assert_eq!(store.revoke(&first.id).unwrap().configuration_revision, 4);
    assert_eq!(store.revoke(&first.id).unwrap().configuration_revision, 4);
    let unused = store.create_token(3600).unwrap().token;
    let third = self::request("three");
    assert!(matches!(
        store.enroll(&unused, &third),
        Err(Error::Capacity(_))
    ));
    let binding: Option<String> = store
        .connection
        .query_row(
            "SELECT used_node_id FROM enrollment_tokens WHERE token_hash = ?1",
            [local::digest(&unused)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(binding, None);
    assert_eq!(store.count().unwrap(), 2);
    assert_eq!(store.revision().unwrap(), 4);
    assert!(matches!(
        store.enroll(&unused, &third),
        Err(Error::Capacity(_))
    ));
    let before = serde_json::to_value(store.nodes().unwrap()).unwrap();
    drop(store);
    let reopened = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(
        reopened
            .configuration(&second_request.node_credential)
            .unwrap()
            .payload
            .network,
        options_pool(&options)
    );
    assert_eq!(
        serde_json::to_value(reopened.nodes().unwrap()).unwrap(),
        before
    );
    drop(reopened);
    assert!(
        SingleNetworkFixture::open(
            dir.path(),
            &NetworkOptions {
                ipv4_prefix: Some("100.97.0.0/16".into()),
                ipv6_prefix: None
            }
        )
        .is_err()
    );
    assert!(SingleNetworkFixture::open(dir.path(), &options).is_ok());
}

fn options_pool(options: &NetworkOptions) -> MeshNetwork {
    MeshNetwork {
        ipv4_prefix: options.ipv4_prefix.clone().unwrap(),
        ipv6_prefix: options.ipv6_prefix.clone().unwrap(),
    }
}
