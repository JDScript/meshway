use super::*;

#[tokio::test]
#[ignore = "two isolated Linux namespaces sharing a private journal directory and NET_ADMIN"]
async fn container_restart_archives_previous_namespace_journal() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let phase = std::env::var("MESHWAY_RESTART_PHASE").unwrap();
    let directory =
        Arc::new(Directory::open(std::path::Path::new("/restart/node"), "fixture.lock").unwrap());
    let previous = if phase == "b" {
        Some(std::fs::read(directory.path().join("underlay.json")).unwrap())
    } else {
        assert_eq!(phase, "a");
        None
    };
    let tasks = Tasks::new(64);
    let mut kernel = Kernel::open(directory.clone(), Options::default(), tasks.clone())
        .await
        .unwrap();
    if let Some(bytes) = previous {
        let old: Journal = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(old.boot, kernel.journal.boot);
        assert_ne!(old.namespace_inode, kernel.journal.namespace_inode);
        assert_ne!(old.installation, kernel.journal.installation);
        let archives: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("underlay.json.stale-")
            })
            .collect();
        assert_eq!(archives.len(), 1);
        assert_eq!(std::fs::read(&archives[0]).unwrap(), bytes);
        assert!(kernel.journal.rules.is_empty());
        assert!(kernel.shared.execution.read().unwrap().is_none());
    }
    kernel
        .retain_negative(&["100.96.0.0/16".parse().unwrap()])
        .await
        .unwrap();
    kernel.suspend().await.unwrap();
    assert!(!kernel.journal.rules.is_empty());
    if phase == "b" {
        kernel.down().await.unwrap();
    }
    kernel.settle().await.unwrap();
    drop(kernel);
    tasks.shutdown().await.unwrap();
}
