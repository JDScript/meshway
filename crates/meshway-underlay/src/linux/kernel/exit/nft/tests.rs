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

#[tokio::test]
#[ignore = "requires isolated Linux namespace, nft and NET_ADMIN"]
async fn exit_nft_timed_set_expiry_uses_seconds_and_does_not_revive_elements() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let mut commands = Commands::default();
    let name = format!("meshway_probe_timeout_{}", std::process::id());
    verify_absent(&mut commands, &name).await.unwrap();
    let mut batch = probe_batch(&name);
    let delete = batch["nftables"].as_array_mut().unwrap().pop().unwrap();
    submit(&mut commands, &batch).await.unwrap();
    let initial = elements(&mut commands, &name).await;
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    let expired = elements(&mut commands, &name).await;
    submit(&mut commands, &json!({"nftables": [delete]}))
        .await
        .unwrap();
    verify_absent(&mut commands, &name).await.unwrap();
    assert_eq!(initial, 1);
    assert_eq!(expired, 0);
}

async fn elements(commands: &mut Commands, name: &str) -> usize {
    let output = commands
        .run(
            Tool::Nft,
            vec![
                "-j".into(),
                "list".into(),
                "set".into(),
                "inet".into(),
                name.into(),
                "flows".into(),
            ],
            Vec::new(),
        )
        .await
        .unwrap();
    assert!(output.success);
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let sets: Vec<_> = value["nftables"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry.get("set"))
        .collect();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0]["name"], "flows");
    sets[0]
        .get("elem")
        .map_or(0, |entries| entries.as_array().unwrap().len())
}

#[tokio::test]
#[ignore = "requires isolated Linux namespace, nft and NET_ADMIN"]
async fn exit_nft_exclusive_create_preserves_colliding_foreign_table() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let mut commands = Commands::default();
    let name = format!("meshway_probe_sentinel_{}", std::process::id());
    verify_absent(&mut commands, &name).await.unwrap();
    submit(
        &mut commands,
        &json!({"nftables": [{"create": {"table": {
            "family": "inet", "name": name, "comment": "external sentinel"
        }}}]}),
    )
    .await
    .unwrap();
    let before = table(&mut commands, &name).await;
    assert!(submit(&mut commands, &probe_batch(&name)).await.is_err());
    assert_eq!(
        verify_absent(&mut commands, &name)
            .await
            .unwrap_err()
            .reason,
        Reason::OwnershipConflict
    );
    let after = table(&mut commands, &name).await;
    // Fixture cleanup is explicit; the production probe never deletes the
    // colliding resource, even though a later command could address its name.
    submit(
        &mut commands,
        &json!({"nftables": [{"delete": {"table": {"family": "inet", "name": name}}}]}),
    )
    .await
    .unwrap();
    assert_eq!(before, after);
}

async fn table(commands: &mut Commands, name: &str) -> Value {
    let output = commands
        .run(
            Tool::Nft,
            vec![
                "-j".into(),
                "list".into(),
                "table".into(),
                "inet".into(),
                name.into(),
            ],
            Vec::new(),
        )
        .await
        .unwrap();
    assert!(output.success);
    serde_json::from_slice(&output.stdout).unwrap()
}
