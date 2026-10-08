use super::*;

#[test]
fn readback_requires_schema_and_rejects_colliding_names_without_deletion() {
    let metadata =
        json!({"metainfo": {"version": "1.0.9", "release_name": "test", "json_schema_version": 1}});
    let own = json!({"table": {"family": "inet", "name": "meshway_probe_test", "handle": 7}});
    let encode = |entries| serde_json::to_vec(&json!({"nftables": entries})).unwrap();
    assert!(absent(&encode(vec![metadata.clone()]), "meshway_probe_test").is_ok());
    assert_eq!(
        absent(&encode(vec![metadata.clone(), own]), "meshway_probe_test")
            .unwrap_err()
            .reason,
        Reason::OwnershipConflict
    );
    assert!(absent(br#"{"nftables":[]}"#, "meshway_probe_test").is_err());
    assert!(
        absent(
            &encode(vec![metadata, json!({"flowtable": {}})]),
            "meshway_probe_test"
        )
        .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated Linux namespace, nft and NET_ADMIN"]
async fn exit_nft_atomic_batch_readback_and_failed_transaction_leave_no_table() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let mut commands = Commands::default();
    let name = format!("meshway_probe_test_{}", std::process::id());
    verify_absent(&mut commands, &name).await.unwrap();
    submit(&mut commands, &probe_batch(&name)).await.unwrap();
    verify_absent(&mut commands, &name).await.unwrap();
    let mut batch = probe_batch(&name);
    batch["nftables"].as_array_mut().unwrap().insert(
        2,
        json!({"add": {"rule": {
            "family": "inet", "table": name, "chain": "does_not_exist", "expr": [{"drop": null}]
        }}}),
    );
    assert!(submit(&mut commands, &batch).await.is_err());
    verify_absent(&mut commands, &name).await.unwrap();
}
