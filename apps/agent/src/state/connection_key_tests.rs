use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use super::{Agent, AgentError, connection_key::KeyStore, local, write_identity};

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

fn enrolled(path: &std::path::Path) -> Agent {
    let mut agent = Agent::prepare_enrollment(
        path,
        "http://127.0.0.1:1/",
        &local::credential().unwrap(),
        "connection-key",
    )
    .unwrap();
    agent.identity.node_id = Some(local::credential().unwrap());
    write_identity(path, &agent.identity, true).unwrap();
    agent
}

#[test]
fn sidecar_is_private_stable_and_does_not_rewrite_enrollment() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let agent = enrolled(&path);
    let before = std::fs::read(&path).unwrap();
    let key_path = super::connection_key::path(&path);
    let mut keys = KeyStore::load(&path, &agent.identity).unwrap();
    assert!(keys.public_key().is_none());
    let public = keys.prepare(&path, &agent.identity).unwrap().to_owned();
    assert!(meshway_protocol::valid_connection_public_key(&public));
    let key_bytes = std::fs::read(&key_path).unwrap();
    let inode = std::fs::metadata(&key_path).unwrap().ino();
    assert_eq!(std::fs::metadata(&key_path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(keys.prepare(&path, &agent.identity).unwrap(), public);
    drop(agent);
    let restarted = Agent::load(&path).unwrap();
    let mut keys = KeyStore::load(&path, &restarted.identity).unwrap();
    assert_eq!(keys.prepare(&path, &restarted.identity).unwrap(), public);
    assert_eq!(std::fs::read(&key_path).unwrap(), key_bytes);
    assert_eq!(std::fs::metadata(&key_path).unwrap().ino(), inode);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn sidecar_committed_not_durable_retains_key_and_blocks_publication_across_restart() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let agent = enrolled(&path);
    let key_path = super::connection_key::path(&path);
    let mut keys = KeyStore::load(&path, &agent.identity).unwrap();
    let fault = local::test_faults::DirectorySyncFailure::after_publication(&key_path);
    assert!(matches!(
        keys.prepare(&path, &agent.identity),
        Err(AgentError::Write(
            local::AtomicWriteError::CommittedNotDurable(_)
        ))
    ));
    let before = std::fs::read(&key_path).unwrap();
    let public = keys.public_key().unwrap().to_owned();
    assert!(matches!(
        keys.prepare(&path, &agent.identity),
        Err(AgentError::File(_))
    ));
    let mut recovered = KeyStore::load(&path, &agent.identity).unwrap();
    assert_eq!(recovered.public_key().unwrap(), public);
    assert!(matches!(
        recovered.prepare(&path, &agent.identity),
        Err(AgentError::File(_))
    ));
    drop(fault);
    assert_eq!(recovered.prepare(&path, &agent.identity).unwrap(), public);
    assert_eq!(std::fs::read(&key_path).unwrap(), before);
}

#[test]
fn sidecar_rejects_replacement_permissions_links_and_binding_or_key_mismatch() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let agent = enrolled(&path);
    let key_path = super::connection_key::path(&path);
    let mut keys = KeyStore::load(&path, &agent.identity).unwrap();
    keys.prepare(&path, &agent.identity).unwrap();
    let original = std::fs::read(&key_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&original).unwrap();
    let private = parsed["private_key"].as_str().unwrap();

    for (field, value) in [
        ("controller", "http://different.invalid".into()),
        ("node_id", local::credential().unwrap()),
        ("public_key", "0".repeat(64)),
        ("private_key", "0".repeat(64)),
    ] {
        let mut bad = parsed.clone();
        bad[field] = value.into();
        let bad = serde_json::to_vec(&bad).unwrap();
        local::atomic_write(&key_path, &bad, true).unwrap();
        let error = keys.prepare(&path, &agent.identity).unwrap_err();
        assert!(!error.to_string().contains(private));
        assert!(!format!("{error:?}").contains(private));
        assert!(KeyStore::load(&path, &agent.identity).is_err());
        assert_eq!(std::fs::read(&key_path).unwrap(), bad);
    }
    local::atomic_write(&key_path, private.as_bytes(), true).unwrap();
    let error = keys.prepare(&path, &agent.identity).unwrap_err();
    assert!(!format!("{error:?}").contains(private));
    assert_eq!(std::fs::read(&key_path).unwrap(), private.as_bytes());

    // Even another internally valid key must not replace the in-memory binding.
    let other_path = directory.path().join("other.json");
    local::atomic_write(&other_path, &std::fs::read(&path).unwrap(), false).unwrap();
    let mut other = KeyStore::load(&other_path, &agent.identity).unwrap();
    other.prepare(&other_path, &agent.identity).unwrap();
    let replacement = std::fs::read(super::connection_key::path(&other_path)).unwrap();
    local::atomic_write(&key_path, &replacement, true).unwrap();
    assert!(keys.prepare(&path, &agent.identity).is_err());
    assert_eq!(std::fs::read(&key_path).unwrap(), replacement);
    std::fs::remove_file(&key_path).unwrap();
    assert!(keys.prepare(&path, &agent.identity).is_err());
    assert!(!key_path.exists());

    local::atomic_write(&key_path, &original, false).unwrap();
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(keys.prepare(&path, &agent.identity).is_err());
    assert!(KeyStore::load(&path, &agent.identity).is_err());
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let linked = directory.path().join("linked.json");
    std::fs::hard_link(&key_path, &linked).unwrap();
    assert!(keys.prepare(&path, &agent.identity).is_err());
    std::fs::remove_file(&linked).unwrap();
    std::fs::rename(&key_path, &linked).unwrap();
    symlink(&linked, &key_path).unwrap();
    assert!(keys.prepare(&path, &agent.identity).is_err());
    assert_eq!(std::fs::read(&linked).unwrap(), original);
}

#[cfg(feature = "managed-connections")]
#[test]
fn managed_export_reuses_bound_key_and_rechecks_durability_and_binding() {
    let directory = directory();
    let path = directory.path().join("agent.json");
    let agent = enrolled(&path);
    let key_path = super::connection_key::path(&path);
    let mut keys = KeyStore::load(&path, &agent.identity).unwrap();
    assert!(keys.application_key(&path, &agent.identity).is_err());
    let public = keys.prepare(&path, &agent.identity).unwrap().to_owned();
    let before = std::fs::read(&key_path).unwrap();
    let exported = keys.application_key(&path, &agent.identity).unwrap();
    assert_eq!(
        meshway_dataplane::Identity::from_private_key(exported).public_key_hex(),
        public
    );
    assert_eq!(std::fs::read(&key_path).unwrap(), before);
    let fault = local::test_faults::DirectorySyncFailure::after_publication(&key_path);
    assert!(local::atomic_write(&key_path, &before, true).is_err());
    assert!(keys.application_key(&path, &agent.identity).is_err());
    drop(fault);
    let mut replaced: serde_json::Value = serde_json::from_slice(&before).unwrap();
    replaced["node_id"] = local::credential().unwrap().into();
    local::atomic_write(&key_path, &serde_json::to_vec(&replaced).unwrap(), true).unwrap();
    assert!(keys.application_key(&path, &agent.identity).is_err());
    assert_ne!(std::fs::read(&key_path).unwrap(), before);
}
