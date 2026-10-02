use meshway_protocol::*;

use super::super::test_support::{SingleNetworkFixture, directory, enroll, request};
use crate::{Error, NetworkOptions};

#[test]
fn global_cas_normalized_noop_and_zone_withdrawal_are_durable() {
    let dir = directory();
    let mut store = SingleNetworkFixture::open(dir.path(), &NetworkOptions::default()).unwrap();
    let req = request("BÜCHER");
    enroll(&mut store, &req);
    let initial = store.network().unwrap();
    let mut save = SaveNetworkRequest {
        expected_revision: initial.revision,
        name: "网络".into(),
        dns: MeshDnsConfiguration {
            enabled: true,
            zone: "BÜCHER.Test.".into(),
            ttl_seconds: 30,
        },
    };
    let saved = store.save_network(&save).unwrap();
    assert_eq!(saved.revision, initial.revision + 1);
    assert_eq!(saved.network, initial.network);
    let enabled = store.agent_dns(&req.node_credential).unwrap();
    assert_eq!(enabled.payload.records.len(), 1);
    assert_eq!(
        enabled.payload.records[0].aliases,
        ["xn--bcher-kva.xn--bcher-kva.test"]
    );
    assert!(matches!(store.save_network(&save), Err(Error::Conflict)));
    save.expected_revision = saved.revision;
    assert_eq!(store.save_network(&save).unwrap().revision, saved.revision);
    assert_eq!(
        store.agent_dns(&req.node_credential).unwrap().digest,
        enabled.digest
    );
    save.dns.zone = "new.test".into();
    let changed = store.save_network(&save).unwrap();
    let moved = store.agent_dns(&req.node_credential).unwrap();
    assert_ne!(enabled.digest, moved.digest);
    assert!(
        moved
            .payload
            .records
            .iter()
            .all(|r| r.canonical_name.ends_with(".new.test"))
    );
    save.expected_revision = changed.revision;
    save.dns.enabled = false;
    let disabled = store.save_network(&save).unwrap();
    assert_eq!(disabled.revision, changed.revision + 1);
    let withdrawn = store.agent_dns(&req.node_credential).unwrap();
    assert!(withdrawn.payload.records.is_empty());
    assert_ne!(withdrawn.digest, moved.digest);
    save.expected_revision = disabled.revision;
    save.name = "display rename".into();
    store.save_network(&save).unwrap();
    assert_ne!(
        store.agent_dns(&req.node_credential).unwrap().digest,
        withdrawn.digest
    );
    // Other global mutations make network CAS stale as well.
    enroll(&mut store, &request("peer"));
    assert!(matches!(store.save_network(&save), Err(Error::Conflict)));
}
