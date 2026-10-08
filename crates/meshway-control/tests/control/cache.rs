use super::{Control, Server};
use meshway_configuration::execution::CompiledNode;
use meshway_control::Error;
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

fn old_slice(current: &CompiledNode) -> String {
    // This is the exact prior serialization order, obtained from the production
    // compiler output by removing only the three new derived contract fields.
    let cfg = &current.configuration;
    let mut body = serde_json::to_string(cfg).unwrap();
    body = body.replace(",\"source\":\"configured\"", "");
    for (name, value) in [
        (
            "meshAddressPools",
            serde_json::to_string(&cfg.mesh_address_pools).unwrap(),
        ),
        (
            "localEgress",
            serde_json::to_string(&cfg.local_egress).unwrap(),
        ),
        ("egress", serde_json::to_string(&cfg.dns.egress).unwrap()),
    ] {
        let field = format!(",\"{name}\":{value}");
        assert_eq!(body.matches(&field).count(), 1);
        body = body.replacen(&field, "", 1);
    }
    let digest = meshway_model::Digest::from_bytes(Sha256::digest(body.as_bytes()).into());
    format!(r#"{{"digest":"{digest}","configuration":{body}}}"#)
}

fn record(bytes: &[u8], version: u8, saved: Option<String>) -> Vec<u8> {
    let mut fields: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(bytes).unwrap();
    fields.insert(
        "version".into(),
        RawValue::from_string(version.to_string()).unwrap(),
    );
    if let Some(saved) = saved {
        fields.insert("saved".into(), RawValue::from_string(saved).unwrap());
    }
    serde_json::to_vec(&fields).unwrap()
}

fn altered_old_slice(old: &str, change: impl FnOnce(String) -> String) -> String {
    let mut slice: BTreeMap<String, Box<RawValue>> = serde_json::from_str(old).unwrap();
    let body = change(slice["configuration"].get().to_owned());
    let digest = meshway_model::Digest::from_bytes(Sha256::digest(body.as_bytes()).into());
    slice.insert("configuration".into(), RawValue::from_string(body).unwrap());
    slice.insert(
        "digest".into(),
        RawValue::from_string(serde_json::to_string(&digest).unwrap()).unwrap(),
    );
    serde_json::to_string(&slice).unwrap()
}

#[tokio::test]
async fn known_old_cache_retires_without_reenrollment_and_archives_exact_private_record() {
    upgrade(1).await;
}

#[tokio::test]
async fn v2_cache_retires_without_reenrollment_for_endpoint_source_upgrade() {
    upgrade(2).await;
}

async fn upgrade(version: u8) {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("cache-upgrade");
    let mut control = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let binding = server.enroll(&owner, &mut control, "cache-upgrade").await;
    control.authenticate().await.unwrap();
    let first = control.acquire_instance(BTreeMap::new()).await.unwrap();
    control.sync_configuration().await.unwrap();
    control.acknowledge_saved().await.unwrap();
    let old = if version == 1 {
        old_slice(control.cached_target().unwrap())
    } else {
        let body = serde_json::to_string(&control.cached_target().unwrap().configuration)
            .unwrap()
            .replace(",\"source\":\"configured\"", "");
        let digest = meshway_model::Digest::from_bytes(Sha256::digest(body.as_bytes()).into());
        format!(r#"{{"digest":"{digest}","configuration":{body}}}"#)
    };
    control.shutdown().await.unwrap();
    let current = fs::read(path.join("control.json")).unwrap();
    let bytes = record(&current, version, Some(old));
    fs::write(path.join("control.json"), &bytes).unwrap();
    let intent = server.path("daemon-up-intent");
    fs::write(&intent, b"retained host intent evidence").unwrap();
    let marker = fs::read(path.join("node-control.lock")).unwrap();

    let mut restored = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert_eq!(restored.status().binding, Some(binding.clone()));
    assert!(restored.cached_target().is_none());
    assert!(!restored.status().authenticated);
    assert!(restored.status().instance.is_none());
    let upgraded: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("control.json")).unwrap()).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for field in ["origin", "keys", "enrollment", "binding"] {
        assert_eq!(upgraded[field], original[field]);
    }
    assert_eq!(upgraded["version"], 3);
    assert!(upgraded["saved"].is_null());
    assert_eq!(fs::read(path.join("node-control.lock")).unwrap(), marker);
    assert_eq!(fs::read(&intent).unwrap(), b"retained host intent evidence");
    let hash = meshway_model::Digest::from_bytes(Sha256::digest(&bytes).into());
    let archive = path.join(format!("control-v{version}-{hash}.json"));
    assert_eq!(fs::read(&archive).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&archive).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    restored.authenticate().await.unwrap();
    let fresh = restored.acquire_instance(BTreeMap::new()).await.unwrap();
    assert_ne!(fresh.instance_id, first.instance_id);
    restored.sync_configuration().await.unwrap();
    restored.acknowledge_saved().await.unwrap();
    assert_eq!(restored.status().binding, Some(binding));
    restored.shutdown().await.unwrap();
    // A completed upgrade does not produce another backup on ordinary open.
    let count = fs::read_dir(&path).unwrap().count();
    Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder().unwrap(),
    )
    .unwrap()
    .shutdown()
    .await
    .unwrap();
    assert_eq!(fs::read_dir(&path).unwrap().count(), count);
    server.close().await;
}

#[tokio::test]
async fn unknown_corrupt_and_misbound_cache_records_are_retained_byte_for_byte() {
    let server = Server::start().await;
    let owner = server.owner().await;
    let path = server.path("bad-cache");
    let mut control = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    server.enroll(&owner, &mut control, "bad-cache").await;
    control.authenticate().await.unwrap();
    control.acquire_instance(BTreeMap::new()).await.unwrap();
    control.sync_configuration().await.unwrap();
    let old = old_slice(control.cached_target().unwrap());
    control.shutdown().await.unwrap();
    let current = fs::read(path.join("control.json")).unwrap();
    let mut bad_identity: serde_json::Value = serde_json::from_slice(&current).unwrap();
    let mut corrupt_current: serde_json::Value = serde_json::from_slice(&current).unwrap();
    corrupt_current["saved"]["digest"] = serde_json::json!("00".repeat(32));
    bad_identity["keys"]["control"] = serde_json::json!("bad private key");
    let bad_identity = record(
        &serde_json::to_vec(&bad_identity).unwrap(),
        1,
        Some(old.clone()),
    );
    let mut bad_digest: serde_json::Value = serde_json::from_str(&old).unwrap();
    bad_digest["digest"] = serde_json::json!("00".repeat(32));
    let duplicate = altered_old_slice(&old, |body| {
        body.replacen("\"networkId\":", "\"networkId\":null,\"networkId\":", 1)
    });
    let bad_shape = altered_old_slice(&old, |body| {
        body.replacen("\"enabled\":true", "\"enabled\":\"true\"", 1)
    });
    let wrong_node = altered_old_slice(&old, |body| {
        let node = serde_json::from_str::<serde_json::Value>(&body).unwrap()["nodeId"].clone();
        let replacement = super::support::request().to_string();
        body.replacen(
            &format!("\"nodeId\":{node}"),
            &format!("\"nodeId\":\"{replacement}\""),
            1,
        )
    });
    let unknown_root = String::from_utf8(record(&current, 1, Some(old.clone())))
        .unwrap()
        .replacen('{', "{\"unknown\":true,", 1)
        .into_bytes();
    let duplicate_root = String::from_utf8(record(&current, 1, Some(old.clone())))
        .unwrap()
        .replacen('{', "{\"version\":1,", 1)
        .into_bytes();
    for invalid in [
        record(&current, 99, Some(old.clone())),
        serde_json::to_vec(&corrupt_current).unwrap(),
        record(&current, 3, Some(old.clone())), // malformed CURRENT never means obsolete
        record(&current, 2, Some(old.clone())), // v2 cannot contain a v1 slice shape
        record(
            &current,
            1,
            Some(serde_json::to_string(&bad_digest).unwrap()),
        ),
        record(&current, 1, Some(duplicate)),
        record(&current, 1, Some(bad_shape)),
        record(&current, 1, Some(wrong_node)),
        bad_identity,
        unknown_root,
        duplicate_root,
    ] {
        fs::write(path.join("control.json"), &invalid).unwrap();
        let count = fs::read_dir(&path).unwrap().count();
        assert!(matches!(
            Control::open(
                &path,
                &server.origin,
                meshway_control_client::http_client_builder().unwrap()
            ),
            Err(Error::Record)
        ));
        assert_eq!(fs::read(path.join("control.json")).unwrap(), invalid);
        assert_eq!(fs::read_dir(&path).unwrap().count(), count);
    }
    server.close().await;
}

#[tokio::test]
async fn pending_v1_enrollment_and_archive_failure_preserve_identity_and_marker() {
    let server = Server::start().await;
    let path = server.path("pending-upgrade");
    let mut control = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    let pending = control.begin_enrollment("pending-upgrade").await.unwrap();
    control.shutdown().await.unwrap();
    let bytes = record(&fs::read(path.join("control.json")).unwrap(), 1, None);
    fs::write(path.join("control.json"), &bytes).unwrap();
    let marker = fs::read(path.join("node-control.lock")).unwrap();
    let hash = meshway_model::Digest::from_bytes(Sha256::digest(&bytes).into());
    let archive = path.join(format!("control-v1-{hash}.json"));
    fs::create_dir(&archive).unwrap();
    assert!(
        Control::open(
            &path,
            &server.origin,
            meshway_control_client::http_client_builder().unwrap()
        )
        .is_err()
    );
    assert_eq!(fs::read(path.join("control.json")).unwrap(), bytes);
    assert_eq!(fs::read(path.join("node-control.lock")).unwrap(), marker);
    fs::remove_dir(&archive).unwrap();
    // A crash after archiving but before publishing the upgraded record reuses
    // the same checked backup rather than accumulating another secret copy.
    fs::write(&archive, &bytes).unwrap();
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o600)).unwrap();
    let mut restored = Control::open(
        &path,
        &server.origin,
        meshway_control_client::http_client_builder()
            .unwrap()
            .no_proxy(),
    )
    .unwrap();
    assert_eq!(restored.status().enrollment_request_id, Some(pending.id));
    assert_eq!(
        restored
            .begin_enrollment("pending-upgrade")
            .await
            .unwrap()
            .id,
        pending.id
    );
    let before: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("control.json")).unwrap()).unwrap();
    for field in ["keys", "origin", "enrollment", "binding"] {
        assert_eq!(before[field], after[field]);
    }
    restored.shutdown().await.unwrap();
    server.close().await;
}
