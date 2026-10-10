//! Restart: a sealed, live `netmap.json` is authority without the Server;
//! expired, forged or foreign files are not.
use super::{
    HOUR, enrolled,
    support::{Facts, compiled, full, unchanged},
};
use crate::{Control, Server, faults::Proxy};
use meshway_control::NetmapChange;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Duration};

fn reopen(path: &Path, origin: &str) -> Control {
    Control::open(
        path,
        origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap()
}

#[tokio::test]
async fn restart_loads_a_live_sealed_netmap_without_any_server_contact() {
    let server = Server::start().await;
    let proxy = Proxy::start(&server.origin).await;
    let (mut control, binding, script) = enrolled(&server, &proxy, "restart").await;
    let netmap = compiled(&binding, "restart", None);
    script.push(Ok(full(&binding, &netmap, HOUR)));
    let before = control.sync(&mut Facts::default()).await.unwrap().snapshot;
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
    let snapshot = restored.netmap_snapshot().unwrap();
    assert_eq!(snapshot.reference(), netmap.reference());
    assert_eq!(snapshot.expires_at_ms(), before.expires_at_ms());
    assert!(!snapshot.same_netmap(&before) && before.ensure_current().is_err());
    assert_eq!(snapshot.instance(), None);
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
    assert!(restored.netmap_snapshot().is_err());
    assert_eq!(restored.status().netmap, None);
    assert_eq!(restored.netmap_reference(), Some(netmap.reference()));
    restored.authenticate().await.unwrap();
    restored.set_sync_transport(script.clone());
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let outcome = restored.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::Reissued);
    assert_eq!(script.requests()[1].netmap, Some(netmap.reference()));
    outcome.snapshot.ensure_current().unwrap();
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
        assert!(restored.netmap_snapshot().is_err());
        assert_eq!(restored.netmap_reference(), None);
        // A derived cache never blocks the identity.
        assert_eq!(restored.status().binding.as_ref(), Some(&binding));
        restored.shutdown().await.unwrap();
    }
    fs::write(&file, serde_json::to_vec(&original).unwrap()).unwrap();
    let restored = reopen(&path, &proxy.origin);
    restored.netmap_snapshot().unwrap();
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
    assert!(restored.netmap_snapshot().is_err());
    assert_eq!(restored.netmap_reference(), Some(netmap.reference()));
    restored.authenticate().await.unwrap();
    restored.set_sync_transport(script.clone());
    script.push(Ok(unchanged(&binding, &netmap, HOUR)));
    let outcome = restored.sync(&mut Facts::default()).await.unwrap();
    assert_eq!(outcome.change, NetmapChange::Reissued);
    restored.shutdown().await.unwrap();
    // The first signed renewal rewrote the file as format 2 authority.
    let cache: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(cache["format"], "meshway-netmap-cache/2");
    let restored = reopen(&path, &proxy.origin);
    restored.netmap_snapshot().unwrap();
    restored.shutdown().await.unwrap();
    server.close().await;
}
