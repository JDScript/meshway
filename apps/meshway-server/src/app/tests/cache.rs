//! Cache upgrades run against real enrolled identities, publications, receipts
//! and the actual App startup owner. Only the private derived body is aged.
use super::{
    node::{caps, fixture, session},
    *,
};
use crate::{
    accounts::{Secret, random_uuid},
    error::Error,
    store,
};
use meshway_configuration::execution::{CompiledNode, CompiledPublication};
use meshway_model::{
    ConfigurationRef, Digest, InstanceId, NetworkId, NodeId, PublicationId, RequestId, Sequence,
    Version,
};
use meshway_protocol::{execution::*, node::NodeSessionResponse};
use rusqlite::{Connection, params, types::Value};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}
fn instance(lease: &InstanceLease) -> InstanceRequest {
    InstanceRequest {
        instance_id: lease.instance_id,
        generation: lease.generation,
    }
}
async fn acquire(app: &App, token: &NodeSessionResponse) -> InstanceLease {
    app.execution()
        .acquire(
            token.token.expose(),
            AcquireInstanceRequest {
                instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                capabilities: caps(),
            },
        )
        .await
        .unwrap()
}

fn old_slice(current: &CompiledNode) -> (String, ConfigurationRef) {
    let cfg = &current.configuration;
    let mut body = pre_exit_body(current);
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
    let digest = Digest::from_bytes(Sha256::digest(body.as_bytes()).into());
    (
        format!(r#"{{"digest":"{digest}","configuration":{body}}}"#),
        ConfigurationRef {
            revision: cfg.revision,
            execution_publication: cfg.publication,
            digest,
        },
    )
}

fn pre_exit_body(current: &CompiledNode) -> String {
    let mut fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_str(&serde_json::to_string(&current.configuration).unwrap()).unwrap();
    fields.remove("egressCapture").unwrap();
    fields.remove("exitRole");
    // These fixtures carry only Mesh permits; remove the newly explicit null.
    serde_json::to_string(&fields)
        .unwrap()
        .replace(",\"exit\":null", "")
}

fn old_publication(
    current: &CompiledPublication,
    version: &str,
) -> (Vec<u8>, BTreeMap<NodeId, ConfigurationRef>) {
    let mut fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_slice(&serde_json::to_vec(current).unwrap()).unwrap();
    let mut nodes = BTreeMap::new();
    let mut references = BTreeMap::new();
    for (node, slice) in &current.nodes {
        let (raw, reference) = old_slice(slice);
        nodes.insert(*node, RawValue::from_string(raw).unwrap());
        references.insert(*node, reference);
    }
    fields.insert(
        "nodes".into(),
        RawValue::from_string(serde_json::to_string(&nodes).unwrap()).unwrap(),
    );
    fields.insert(
        "compilerVersion".into(),
        RawValue::from_string(serde_json::to_string(version).unwrap()).unwrap(),
    );
    (serde_json::to_vec(&fields).unwrap(), references)
}

/// demand-3/4 use the previous endpoint shape and retain all execution fields.
fn same_shape_publication(
    current: &CompiledPublication,
    version: &str,
) -> (Vec<u8>, BTreeMap<NodeId, ConfigurationRef>) {
    let mut fields: BTreeMap<String, Box<RawValue>> =
        serde_json::from_slice(&serde_json::to_vec(current).unwrap()).unwrap();
    let mut nodes = BTreeMap::new();
    let mut references = BTreeMap::new();
    for (node, slice) in &current.nodes {
        let body = pre_exit_body(slice);
        let body = if version == "meshway-demand-5" {
            body
        } else {
            body.replace(",\"source\":\"configured\"", "")
        };
        let digest = Digest::from_bytes(Sha256::digest(body.as_bytes()).into());
        nodes.insert(
            *node,
            RawValue::from_string(format!(r#"{{"digest":"{digest}","configuration":{body}}}"#))
                .unwrap(),
        );
        references.insert(
            *node,
            ConfigurationRef {
                revision: current.revision,
                execution_publication: current.publication,
                digest,
            },
        );
    }
    fields.insert(
        "nodes".into(),
        RawValue::from_string(serde_json::to_string(&nodes).unwrap()).unwrap(),
    );
    fields.insert(
        "compilerVersion".into(),
        RawValue::from_string(serde_json::to_string(version).unwrap()).unwrap(),
    );
    (serde_json::to_vec(&fields).unwrap(), references)
}

async fn age(
    app: &App,
    network: NetworkId,
    publication: PublicationId,
    version: &str,
) -> BTreeMap<NodeId, ConfigurationRef> {
    let sealer = app.sealer();
    let version = version.to_owned();
    app.executor.handle().execute(move |db| {
        let payload: Vec<u8> = db.query_row("SELECT payload FROM execution_publications WHERE network_id=?1 AND id=?2",
            params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice()], |row| row.get(0)).unwrap();
        let plain = sealer.open_execution(network, publication, &payload).unwrap();
        let compiled: CompiledPublication = serde_json::from_slice(plain.as_bytes()).unwrap();
        let (old, references) = if matches!(version.as_str(), "meshway-demand-3" | "meshway-demand-4" | "meshway-demand-5") {
            same_shape_publication(&compiled, &version)
        } else {
            old_publication(&compiled, &version)
        };
        let sealed = sealer.seal_execution(network, publication, &old).unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("UPDATE execution_publications SET payload=?3 WHERE network_id=?1 AND id=?2",
            params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice(), sealed.as_bytes()]).unwrap();
        // Existing genuine saved receipts refer to the aged fixture's digest.
        for (node, reference) in &references {
            tx.execute("UPDATE execution_receipts SET digest=?4 WHERE network_id=?1 AND publication_id=?2 AND node_id=?3",
                params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), reference.digest.as_bytes().as_slice()]).unwrap();
        }
        tx.commit().unwrap();
        references
    }).await.unwrap()
}

type Rows = Vec<(String, Vec<Vec<Value>>)>;
fn snapshot(db: &Connection, cache: bool) -> Rows {
    let tables = db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([], |row| row.get::<_, String>(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    tables
        .into_iter()
        .filter(|name| {
            cache || !["execution_publications", "node_execution"].contains(&name.as_str())
        })
        .map(|table| {
            let mut statement = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|index| row.get(index))
                        .collect::<Result<Vec<Value>, _>>()
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (table, rows)
        })
        .collect()
}

#[tokio::test]
async fn startup_retires_only_old_cache_preserves_history_and_reauthenticates_same_node() {
    startup_retires_previous_cache("meshway-demand-3").await;
}

#[tokio::test]
async fn startup_retires_demand4_cache_for_endpoint_provenance_without_reenrollment() {
    startup_retires_previous_cache("meshway-demand-4").await;
}

#[tokio::test]
async fn startup_retires_demand5_cache_for_exit_contract_without_reenrollment() {
    startup_retires_previous_cache("meshway-demand-5").await;
}

async fn startup_retires_previous_cache(version: &str) {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    let service = app.execution();
    let at = session(&service, network, a, &ak, &signer_a).await;
    let ai = acquire(&app, &at).await;
    let bt = session(&service, network, b, &bk, &signer_b).await;
    let bi = acquire(&app, &bt).await;
    let ap = ai.target.as_ref().unwrap().execution_publication;
    let bp = bi.target.as_ref().unwrap().execution_publication;
    assert_ne!(ap, bp);
    service
        .receipt(
            at.token.expose(),
            ReceiptRequest {
                instance: instance(&ai),
                attempt: Version::new(1).unwrap(),
                sequence: Sequence::new(1).unwrap(),
                target: ai.target.clone().unwrap(),
                phase: ReceiptPhase::Saved,
                reported_at_ms: store::execution::now().unwrap() as u64,
                failure: None,
            },
        )
        .await
        .unwrap();
    let old_a = age(&app, network, ap, "meshway-demand-1").await[&a].clone();
    age(&app, network, bp, version).await;
    let before = app
        .executor
        .handle()
        .execute(|db| snapshot(db, false))
        .await
        .unwrap();
    let payloads = app
        .executor
        .handle()
        .execute(|db| {
            db.prepare("SELECT id,payload FROM execution_publications ORDER BY id")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .await
        .unwrap();
    app.shutdown().await.unwrap();
    let app = App::open(Settings::new(
        dir.path().canonicalize().unwrap().join("data"),
    ))
    .await
    .unwrap();
    assert_eq!(
        app.executor
            .handle()
            .execute(|db| snapshot(db, false))
            .await
            .unwrap(),
        before
    );
    let sealer = app.sealer();
    app.executor.handle().execute(move |db| {
        let after = db.prepare("SELECT id,payload FROM execution_publications ORDER BY id").unwrap()
            .query_map([], |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?))).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(after, payloads);
        let tx = db.transaction().unwrap();
        for (node, publication) in [(a, ap), (b, bp)] {
            assert!(store::execution::target_for_node(&tx, &sealer, network, node).unwrap().is_none());
            assert!(matches!(store::execution::check_publication(&tx, &sealer, network, node, publication, store::execution::now().unwrap()), Err(Error::BindingConflict)));
            let row: (Option<Vec<u8>>, Option<i64>) = tx.query_row("SELECT target_publication_id,execution_expires_at FROM node_execution WHERE network_id=?1 AND node_id=?2",
                params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
            assert_eq!(row, (None, None));
        }
    }).await.unwrap();
    let service = app.execution();
    assert!(
        service
            .renew(at.token.expose(), instance(&ai))
            .await
            .is_err()
    );
    let at = session(&service, network, a, &ak, &signer_a).await;
    let fresh = acquire(&app, &at).await;
    assert_ne!(fresh.instance_id, ai.instance_id);
    let response = service
        .configuration(
            at.token.expose(),
            NodeConfigurationRequest {
                instance: instance(&fresh),
                base: Some(old_a.clone()),
            },
        )
        .await
        .unwrap();
    assert!(response.delta.is_none());
    let full = response.full.unwrap();
    assert_ne!(full.configuration.publication, ap);
    let serialized = serde_json::to_value(&full.configuration).unwrap();
    assert!(serialized.get("meshAddressPools").is_some());
    assert!(serialized.get("localEgress").is_some());
    assert!(serialized["dns"].get("egress").is_some());
    // The old reference remains valid evidence, without a reconstructed context.
    service
        .receipt(
            at.token.expose(),
            ReceiptRequest {
                instance: instance(&fresh),
                attempt: Version::new(1).unwrap(),
                sequence: Sequence::new(1).unwrap(),
                target: old_a.clone(),
                phase: ReceiptPhase::Saved,
                reported_at_ms: store::execution::now().unwrap() as u64,
                failure: None,
            },
        )
        .await
        .unwrap();
    let token = at.token.expose().to_owned();
    let execution = service.clone();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let admission = execution
                .report_admission_in_transaction(&tx, &token, chrono::Utc::now())
                .unwrap();
            assert!(admission.configurations.contains(&old_a));
            assert!(
                !admission
                    .contexts
                    .iter()
                    .any(|context| context.configuration == old_a)
            );
            assert_ne!(admission.execution.as_ref(), Some(&old_a));
        })
        .await
        .unwrap();
    let owner = app
        .accounts()
        .login(
            "owner@example.com".into(),
            Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap();
    let progress = service
        .progress(owner.token.expose(), network, ap)
        .await
        .unwrap();
    assert!(!progress.nodes[&a].current);
    assert!(!progress.nodes[&a].applied);
    assert_eq!(progress.nodes[&a].freshness, ReceiptFreshness::Superseded);
    app.shutdown().await.unwrap();
}

#[tokio::test]
async fn unknown_corrupt_and_current_schema_mismatch_abort_boot_without_retiring_any_cache() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (dir, app, network, a, _, ak, _, signer, _) = fixture().await;
    let service = app.execution();
    let token = session(&service, network, a, &ak, &signer).await;
    let lease = acquire(&app, &token).await;
    let publication = lease.target.unwrap().execution_publication;
    let original = app
        .executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT payload FROM execution_publications WHERE id=?1",
                [publication.as_uuid().as_bytes().as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
    let sealer = app.sealer();
    let plain = sealer
        .open_execution(network, publication, &original)
        .unwrap();
    let current: CompiledPublication = serde_json::from_slice(plain.as_bytes()).unwrap();
    let (old, _) = old_publication(&current, "meshway-demand-2");
    let mut bad_seal = original.clone();
    *bad_seal.last_mut().unwrap() ^= 1;
    let mut unknown = current.clone();
    unknown.compiler_version = "meshway-demand-999".into();
    let (mismatch, _) = old_publication(&current, meshway_policy::compile::COMPILER_VERSION);
    let mut bad_digest = current.clone();
    bad_digest.nodes.get_mut(&a).unwrap().digest = Digest::from_bytes([0; 32]);
    let mut bad_header = current.clone();
    bad_header.publication = PublicationId::new(random_uuid().unwrap()).unwrap();
    let mut variants = vec![bad_seal];
    let old_bad_digest = String::from_utf8(old.clone())
        .unwrap()
        .replacen("\"enabled\":true", "\"enabled\":false", 1)
        .into_bytes();
    assert_ne!(old_bad_digest, old);
    let duplicate = String::from_utf8(old.clone())
        .unwrap()
        .replacen('{', "{\"compilerVersion\":\"meshway-demand-2\",", 1)
        .into_bytes();
    for raw in [
        serde_json::to_vec(&unknown).unwrap(),
        mismatch,
        serde_json::to_vec(&bad_digest).unwrap(),
        serde_json::to_vec(&bad_header).unwrap(),
        old_bad_digest,
        duplicate,
        b"{".to_vec(),
    ] {
        variants.push(
            sealer
                .seal_execution(network, publication, &raw)
                .unwrap()
                .as_bytes()
                .to_vec(),
        );
    }
    // An otherwise valid old row sorts before the corrupt row. Its retirement
    // must roll back along with the startup owner transaction.
    let earlier = app
        .executor
        .handle()
        .execute(move |db| {
            db.query_row(
                "SELECT id FROM execution_publications WHERE id!=?1 ORDER BY sequence LIMIT 1",
                [publication.as_uuid().as_bytes().as_slice()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap()
        })
        .await
        .unwrap();
    let earlier = PublicationId::new(uuid::Uuid::from_slice(&earlier).unwrap()).unwrap();
    age(&app, network, earlier, "meshway-demand-1").await;
    app.shutdown().await.unwrap();
    let path = dir.path().canonicalize().unwrap().join("data");
    for payload in variants {
        let before = {
            let db = Connection::open(path.join("controller.sqlite")).unwrap();
            db.execute(
                "UPDATE execution_publications SET payload=?2 WHERE id=?1",
                params![publication.as_uuid().as_bytes().as_slice(), payload],
            )
            .unwrap();
            snapshot(&db, true)
        };
        assert!(matches!(
            App::open(Settings::new(&path)).await,
            Err(AppError::StorageIntegrity)
        ));
        let db = Connection::open(path.join("controller.sqlite")).unwrap();
        assert_eq!(snapshot(&db, true), before);
    }
    // Restoring a recognized old envelope permits the ordinary startup path.
    {
        let db = Connection::open(path.join("controller.sqlite")).unwrap();
        let sealed = sealer.seal_execution(network, publication, &old).unwrap();
        db.execute(
            "UPDATE execution_publications SET payload=?2 WHERE id=?1",
            params![
                publication.as_uuid().as_bytes().as_slice(),
                sealed.as_bytes()
            ],
        )
        .unwrap();
    }
    App::open(Settings::new(&path))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn first_use_retires_old_target_before_reading_lease_deadlines() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let (_dir, app, network, node, _, keys, _, signer, _) = fixture().await;
    let token = session(&app.execution(), network, node, &keys, &signer).await;
    let lease = acquire(&app, &token).await;
    assert!(lease.execution_expires_at_ms.is_some());
    let publication = lease.target.as_ref().unwrap().execution_publication;
    age(&app, network, publication, "meshway-demand-2").await;
    let sealer = app.sealer();
    app.executor
        .handle()
        .execute(move |db| {
            let tx = db.transaction().unwrap();
            let after = store::execution::lease_for_node(&tx, &sealer, network, node).unwrap();
            assert_eq!(after.instance_id, lease.instance_id);
            assert_eq!(after.expires_at_ms, lease.expires_at_ms);
            assert!(after.target.is_none());
            assert!(after.execution_expires_at_ms.is_none());
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    let renewed = app
        .execution()
        .renew(token.token.expose(), instance(&lease))
        .await
        .unwrap();
    assert!(renewed.target.is_none() && renewed.execution_expires_at_ms.is_none());
    let fresh = acquire(&app, &token).await;
    assert_ne!(fresh.target.unwrap().execution_publication, publication);
    app.shutdown().await.unwrap();
}
