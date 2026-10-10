//! Restart: a sealed, live `netmap.json` is authority without the Server;
//! expired, forged or foreign files are not.
use super::{
    HOUR, enrolled,
    support::{Facts, compiled, compiled_with_peer, full, unchanged},
};
use crate::{Control, Server, faults::Proxy};
use meshway_control::NetmapChange;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

fn reopen(path: &Path, origin: &str) -> Control {
    Control::open(
        path,
        origin,
        meshway_test_server::client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap()
}

#[tokio::test]
async fn known_identity_rejection_survives_restart_until_a_fresh_signed_sync() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "rejected-cache").await;
    let netmap = compiled(&binding, "rejected-cache", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    script.push(Err(super::review::identity_rejection()));
    assert!(control.sync(&mut Facts::default()).await.is_err());
    assert!(control.netmap().is_none());
    control.shutdown().await.unwrap();
    let path = server.path("rejected-cache");
    let mut restored = reopen(&path, &proxy.origin);
    assert_eq!(restored.netmap_reference(), Some(netmap.reference()));
    assert!(restored.netmap().is_none());
    restored.authenticate().await.unwrap();
    restored.set_sync_transport(script.clone());
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    restored.sync(&mut Facts::default()).await.unwrap();
    assert!(restored.netmap().is_some());
    restored.shutdown().await.unwrap();
    let offline = reopen(&path, &proxy.origin);
    assert!(offline.netmap().is_some());
    offline.shutdown().await.unwrap();
    proxy.close().await;
    server.close().await;
}

#[tokio::test]
async fn restart_loads_a_live_sealed_netmap_without_any_server_contact() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "restart").await;
    let netmap = compiled(&binding, "restart", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let before = control.sync(&mut Facts::default()).await.unwrap().netmap;
    control.shutdown().await.unwrap();
    let path = server.path("restart");
    let origin = proxy.origin.clone();
    let file = path.join("netmap.json");
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("control.json")).unwrap()).unwrap();
    assert_eq!(record["version"], 5);
    assert!(record.get("saved").is_none() && record.get("netmap").is_none());
    // The Server signing key is pinned with the binding.
    assert_eq!(
        record["binding"]["serverSigningKey"],
        serde_json::to_value(super::support::server_key().1).unwrap()
    );
    let cache: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(cache["format"], "meshway-netmap-cache/2");
    assert_eq!(cache["seal"]["signer"], "node");
    assert_eq!(
        cache["signature"]["keyId"],
        serde_json::to_value(super::support::server_key().1.key_id).unwrap()
    );
    // Closing the fixture removes its directory: keep an exact private copy.
    let kept = tempfile::tempdir().unwrap();
    let copy = kept.path().canonicalize().unwrap().join("restart");
    fs::create_dir(&copy).unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700)).unwrap();
    for entry in fs::read_dir(&path).unwrap() {
        let entry = entry.unwrap();
        let target = copy.join(entry.file_name());
        fs::copy(entry.path(), &target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    }
    server.close().await;

    let restored = reopen(&copy, &origin);
    let published = restored.netmap().unwrap();
    assert_eq!(published.content.reference(), netmap.reference());
    assert_eq!(published.lease, before.lease);
    assert!(!restored.status().authenticated);
    assert_eq!(restored.status().netmap.unwrap().netmap, netmap.reference());
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn expired_cache_is_only_a_delta_base_until_the_server_renews_it() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "expired").await;
    let netmap = compiled(&binding, "expired", None);
    script.push(Ok(full(&binding, &netmap, 300)));
    control.sync(&mut Facts::default()).await.unwrap();
    control.shutdown().await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut restored = reopen(&server.path("expired"), &proxy.origin);
    assert!(restored.netmap().is_none());
    assert_eq!(restored.status().netmap, None);
    assert_eq!(restored.netmap_reference(), Some(netmap.reference()));
    restored.authenticate().await.unwrap();
    restored.set_sync_transport(script.clone());
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let outcome = restored.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::Renewed);
    assert_eq!(script.requests()[1].netmap, Some(netmap.reference()));
    assert_eq!(restored.netmap().unwrap().lease, outcome.netmap.lease);
    restored.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn forged_extended_or_altered_netmap_files_are_not_loaded() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "forged").await;
    let netmap = compiled(&binding, "forged", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    control.shutdown().await.unwrap();
    let path = server.path("forged");
    let file = path.join("netmap.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    let mut forgeries = Vec::new();
    let mut signature = original.clone();
    let text = signature["seal"]["signature"].as_str().unwrap().to_owned();
    let flipped = if text.starts_with('A') { "B" } else { "A" };
    signature["seal"]["signature"] = format!("{flipped}{}", &text[1..]).into();
    forgeries.push(signature);
    // A longer lease with the old seal.
    let mut extended = original.clone();
    extended["lease"]["expiresAt"] = (chrono::Utc::now() + chrono::Duration::hours(20))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        .into();
    forgeries.push(extended);
    // Other content with a matching digest and lease, but the old seal.
    let other = compiled(&binding, "forged-other", Some(&netmap));
    let mut altered = original.clone();
    altered["netmap"] = serde_json::to_value(&other).unwrap();
    altered["lease"]["netmap"] = serde_json::to_value(other.reference()).unwrap();
    forgeries.push(altered);
    let mut unknown = original.clone();
    unknown["extra"] = true.into();
    forgeries.push(unknown);
    let mut format = original.clone();
    format["format"] = "meshway-netmap-cache/0".into();
    forgeries.push(format);
    // Server signature tampered, or naming another key, with a valid Node seal.
    let mut tampered = original.clone();
    let text = tampered["signature"]["signature"]
        .as_str()
        .unwrap()
        .to_owned();
    let flipped = if text.starts_with('A') { "B" } else { "A" };
    tampered["signature"]["signature"] = format!("{flipped}{}", &text[1..]).into();
    forgeries.push(tampered);
    let mut key = original.clone();
    key["signature"]["keyId"] = crate::support::request().to_string().into();
    forgeries.push(key);
    // A format 2 file without the Server signature.
    let mut unsigned = original.clone();
    unsigned.as_object_mut().unwrap().remove("signature");
    forgeries.push(unsigned);
    for forged in forgeries {
        fs::write(&file, serde_json::to_vec(&forged).unwrap()).unwrap();
        let restored = reopen(&path, &proxy.origin);
        assert!(restored.netmap().is_none());
        assert_eq!(restored.netmap_reference(), None);
        // A derived cache never blocks the identity.
        assert_eq!(restored.status().binding.as_ref(), Some(&binding));
        restored.shutdown().await.unwrap();
    }
    fs::write(&file, serde_json::to_vec(&original).unwrap()).unwrap();
    let restored = reopen(&path, &proxy.origin);
    assert!(restored.netmap().is_some());
    restored.shutdown().await.unwrap();
    server.close().await;
}

#[tokio::test]
async fn format_1_cache_with_only_the_node_seal_is_a_delta_base_not_authority() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "format-one").await;
    let netmap = compiled(&binding, "format-one", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    control.sync(&mut Facts::default()).await.unwrap();
    control.shutdown().await.unwrap();
    let path = server.path("format-one");
    let file = path.join("netmap.json");
    let mut cache: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(cache["format"], "meshway-netmap-cache/2");
    cache.as_object_mut().unwrap().remove("signature");
    cache["format"] = "meshway-netmap-cache/1".into();
    fs::write(&file, serde_json::to_vec(&cache).unwrap()).unwrap();
    let mut restored = reopen(&path, &proxy.origin);
    assert!(restored.netmap().is_none());
    assert_eq!(restored.netmap_reference(), Some(netmap.reference()));
    restored.authenticate().await.unwrap();
    restored.set_sync_transport(script.clone());
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let outcome = restored.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::Renewed);
    restored.shutdown().await.unwrap();
    // The first signed renewal rewrote the file as format 2 authority.
    let cache: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(cache["format"], "meshway-netmap-cache/2");
    let restored = reopen(&path, &proxy.origin);
    assert!(restored.netmap().is_some());
    restored.shutdown().await.unwrap();
    server.close().await;
}

/// An exact private copy of `path`; closing the fixture removes the original.
fn keep(path: &Path, name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let kept = tempfile::tempdir().unwrap();
    let copy = kept.path().canonicalize().unwrap().join(name);
    fs::create_dir(&copy).unwrap();
    fs::set_permissions(&copy, fs::Permissions::from_mode(0o700)).unwrap();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let target = copy.join(entry.file_name());
        fs::copy(entry.path(), &target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    }
    (kept, copy)
}

fn peer_facts(peer: meshway_model::NodeId, address: &str) -> meshway_protocol::PeerFacts {
    use meshway_model::state::{ListenerPhase, LocalEndpoint, LocalEndpointSource, SocketNetwork};
    let observed = chrono::Utc::now() - chrono::Duration::minutes(10);
    meshway_protocol::PeerFacts {
        node_id: peer,
        listeners: std::collections::BTreeMap::from([(
            "udp".to_owned(),
            meshway_protocol::PeerListenerFacts {
                phase: ListenerPhase::Listening,
                network: SocketNetwork::Udp,
                local_endpoints: vec![LocalEndpoint {
                    address: address.parse().unwrap(),
                    source: LocalEndpointSource::Local,
                }],
                stun_candidates: vec![],
            },
        )]),
        exit: None,
        observed_at: observed,
        // Already stale: expiry is freshness, the facts stay last known.
        expires_at: observed + chrono::Duration::seconds(60),
    }
}

#[tokio::test]
async fn restart_without_the_server_replays_the_last_known_peer_facts() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "facts-restart").await;
    let (netmap, peer) = compiled_with_peer(&binding, "facts-restart");
    let stranger = meshway_test_server::configuration::node(77);
    let epoch = "00000000-0000-4000-8000-0000000000e2".parse().unwrap();
    let cursor = meshway_protocol::StateCursor {
        epoch,
        sequence: meshway_model::Sequence::new(5).unwrap(),
    };
    let mut response = full(&binding, &netmap, HOUR);
    response.peer_facts = Some(meshway_protocol::PeerFactsDelta {
        base: None,
        cursor,
        netmap_serial: netmap.netmap.serial,
        updated: [
            (peer, peer_facts(peer, "10.9.0.9:41641")),
            (stranger, peer_facts(stranger, "10.9.0.77:41641")),
        ]
        .into(),
        removed: vec![],
    });
    script.push(Ok(response));
    control.sync(&mut Facts::default()).await.unwrap();
    control.shutdown().await.unwrap();
    let path = server.path("facts-restart");
    let file = path.join("peer-facts.json");
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["format"], "meshway-peer-facts-cache/1");
    assert_eq!(
        saved["netmap"],
        serde_json::to_value(netmap.reference()).unwrap()
    );
    assert!(
        saved["peers"].get(stranger.to_string()).is_none(),
        "facts outside the projection are not kept"
    );
    let (_kept, copy) = keep(&path, "facts-restart");
    let origin = proxy.origin.clone();
    proxy.close().await;
    server.close().await;

    // Server gone: the saved facts are Control's last known facts again.
    let restored = reopen(&copy, &origin);
    assert!(restored.netmap().is_some());
    let known = restored.known_peers().expect("saved facts");
    assert_eq!(known.netmap, netmap.reference());
    assert_eq!(known.peers.keys().copied().collect::<Vec<_>>(), [peer]);
    assert_eq!(
        known.peers[&peer].listeners["udp"].local_endpoints[0]
            .address
            .to_string(),
        "10.9.0.9:41641"
    );
    restored.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_restarted_servers_complete_facts_update_and_never_erase_last_known_facts() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "facts-full").await;
    let (netmap, peer) = compiled_with_peer(&binding, "facts-full");
    let cursor = |epoch: &str, sequence| meshway_protocol::StateCursor {
        epoch: epoch.parse().unwrap(),
        sequence: meshway_model::Sequence::new(sequence).unwrap(),
    };
    let first = "00000000-0000-4000-8000-0000000000e4";
    let second = "00000000-0000-4000-8000-0000000000e5";
    let mut response = full(&binding, &netmap, HOUR);
    response.peer_facts = Some(meshway_protocol::PeerFactsDelta {
        base: None,
        cursor: cursor(first, 5),
        netmap_serial: netmap.netmap.serial,
        updated: [(peer, peer_facts(peer, "10.9.0.9:41641"))].into(),
        removed: vec![],
    });
    script.push(Ok(response));
    let mut sink = Facts::default();
    control.sync(&mut sink).await.unwrap();
    assert_eq!(sink.published.len(), 1);
    // The Server restarted (new epoch) and knows no facts yet: its complete
    // answer is empty, which is not a removal.
    let mut response = unchanged(&binding, &netmap, HOUR);
    response.peer_facts = Some(meshway_protocol::PeerFactsDelta {
        base: None,
        cursor: cursor(second, 1),
        netmap_serial: netmap.netmap.serial,
        updated: Default::default(),
        removed: vec![],
    });
    script.push(Ok(response));
    control.sync(&mut sink).await.unwrap();
    assert!(control.known_peers().unwrap().peers.contains_key(&peer));
    assert_eq!(
        sink.published.len(),
        1,
        "nothing changed, nothing published"
    );
    // Only `removed` deletes.
    let mut response = unchanged(&binding, &netmap, HOUR);
    response.peer_facts = Some(meshway_protocol::PeerFactsDelta {
        base: Some(cursor(second, 1)),
        cursor: cursor(second, 2),
        netmap_serial: netmap.netmap.serial,
        updated: Default::default(),
        removed: vec![peer],
    });
    script.push(Ok(response));
    control.sync(&mut sink).await.unwrap();
    assert!(control.known_peers().unwrap().peers.is_empty());
    assert!(sink.published[1].peers.is_empty());
    control.shutdown().await.unwrap();
    server.close().await;
}
