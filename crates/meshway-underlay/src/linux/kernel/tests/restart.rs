use super::*;

#[tokio::test]
#[ignore = "two live isolated Linux namespaces sharing journal/signals and NET_ADMIN"]
async fn container_restart_archives_previous_namespace_journal() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let phase = std::env::var("MESHWAY_RESTART_PHASE").unwrap();
    std::fs::write(
        format!("/restart/identity-{phase}.json"),
        serde_json::to_vec(&Journal::identity().unwrap()).unwrap(),
    )
    .unwrap();
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
    if let Some(bytes) = previous.clone() {
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
    if let Some(bytes) = previous {
        // A restarted container may receive the destroyed namespace's nsfs
        // inode. A's journal under B's exact identity names objects B never
        // had: it is archived, not adopted and not a conflict.
        let mut reused: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let (_, device, inode) = Journal::identity().unwrap();
        reused["namespace_device"] = device.into();
        reused["namespace_inode"] = inode.into();
        let reused = serde_json::to_vec(&reused).unwrap();
        directory.replace("underlay.json", &reused).unwrap();
        let tasks = Tasks::new(64);
        let mut kernel = Kernel::open(directory.clone(), Options::default(), tasks.clone())
            .await
            .unwrap();
        let old: Journal = serde_json::from_slice(&reused).unwrap();
        assert_eq!(
            (old.namespace_device, old.namespace_inode),
            (
                kernel.journal.namespace_device,
                kernel.journal.namespace_inode
            )
        );
        assert_ne!(old.installation, kernel.journal.installation);
        assert!(kernel.journal.rules.is_empty());
        let archived: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("underlay.json.stale-")
            })
            .map(|path| std::fs::read(path).unwrap())
            .collect();
        assert_eq!(archived.len(), 2);
        assert!(archived.contains(&reused));
        // The fresh allocation installs and cleans up exactly as usual.
        kernel
            .retain_negative(&["100.96.0.0/16".parse().unwrap()])
            .await
            .unwrap();
        kernel.suspend().await.unwrap();
        kernel.down().await.unwrap();
        kernel.settle().await.unwrap();
        drop(kernel);
        tasks.shutdown().await.unwrap();
    }
    drop(directory);
    if phase == "a" {
        // Release every journal/task owner but pin this namespace while B is
        // created. nsfs inode numbers may be reused after namespace destruction;
        // this test specifically exercises two different namespace identities.
        std::fs::write(
            "/signals/underlay-restart-ready.json",
            b"{\"kind\":\"ready\"}",
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(90), async {
            while !std::path::Path::new("/signals/underlay-restart-stop.json").exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
