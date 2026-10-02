//! Namespace reservation and exact-resource ownership regression cases.
use super::*;
use std::os::unix::fs::DirBuilderExt;

fn state(path: &Path) {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => panic!("{error}"),
    }
    private_directory(path).unwrap();
}

fn binding(path: &Path) -> Preparation<'_> {
    Preparation {
        state_directory: path,
        identity: "review-node",
        network: "review-network",
    }
}

fn capture_prefixes() -> [CapturePrefix<'static>; 2] {
    [
        CapturePrefix {
            prefix: "172.30.247.20/32".parse().unwrap(),
            tun: None,
        },
        CapturePrefix {
            prefix: "fd74:6d77:247::20/128".parse().unwrap(),
            tun: None,
        },
    ]
}

#[tokio::test]
#[ignore = "real kernel reproductions of the three frozen legacy topology states"]
async fn legacy_incomplete_topology_refused() {
    for variant in 0..3 {
        let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut owner = LinuxUnderlay::prepare(binding(directory.path()))
            .await
            .unwrap();
        let table = if variant == 0 {
            owner.journal.reservation.mesh_sink_table.unwrap()
        } else {
            owner.journal.reservation.capture_tables[0]
        };
        owner
            .install_one(Resource::Sink {
                family: 4,
                table,
                prefix: "172.30.247.20/32".into(),
            })
            .await
            .unwrap();
        if variant != 0 {
            owner
                .install_one(Resource::Sink {
                    family: 6,
                    table,
                    prefix: "fd74:6d77:247::20/128".into(),
                })
                .await
                .unwrap();
        }
        if variant == 1 {
            owner
                .install_one(Resource::Capture {
                    family: 4,
                    priority: owner.journal.reservation.first_priority + 1,
                    table,
                })
                .await
                .unwrap();
        }
        assert!(owner.journal.pending.is_none());
        assert!(owner.journal.operation.is_none());
        assert!(owner.socket_generation().await.is_err());
        if variant == 0 {
            assert!(
                owner
                    .retain_mesh_sink(capture_prefixes()[0].prefix)
                    .await
                    .is_err()
            );
        } else {
            assert!(owner.activate_tun(&capture_prefixes()).await.is_err());
        }
        println!(
            "legacy counterexample {variant}: incomplete row-only topology explicitly refused"
        );
        while owner.journal.installed.len() > 4 {
            owner
                .remove_one(owner.journal.installed.len() - 1)
                .await
                .unwrap();
        }
        owner.relinquish().await.unwrap();
    }
}

async fn rejected_lookup(family: u8, forwarded: bool) {
    let (destination, source) = if family == 4 {
        ("172.30.247.20", "172.30.247.21")
    } else {
        ("fd74:6d77:247::20", "fd74:6d77:247::21")
    };
    let mut args = vec![
        format!("-{family}"),
        "route".into(),
        "get".into(),
        destination.into(),
    ];
    if forwarded {
        args.extend(["from".into(), source.into(), "iif".into(), "eth0".into()]);
    }
    assert!(
        ip(args).await.is_err(),
        "local/forwarded lookup must reject after complete Mesh sink: v{family}/{forwarded}"
    );
}

#[tokio::test]
#[ignore = "real isolated Docker netns, cancellation at each completed-resource boundary"]
async fn cancellation_boundaries() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    for (kind, count) in [("mesh", 2), ("capture", 4)] {
        for after in 1..=count {
            let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            let mut owner = LinuxUnderlay::prepare(binding(directory.path()))
                .await
                .unwrap();
            let checkpoint = Arc::new(InstallCheckpoint {
                after,
                crash: false,
                reached: tokio::sync::Notify::new(),
                release: tokio::sync::Notify::new(),
            });
            owner.checkpoint = Some(checkpoint.clone());
            let prefixes = capture_prefixes();
            let mut future = Box::pin(async {
                if kind == "mesh" {
                    owner.retain_mesh_sink(prefixes[0].prefix).await
                } else {
                    owner.capture(&prefixes).await
                }
            });
            tokio::select! {
                _ = checkpoint.reached.notified() => {},
                result = &mut future => panic!("operation completed instead of pausing: {result:?}"),
                _ = tokio::time::sleep(Duration::from_secs(10)) => panic!("checkpoint timeout"),
            }
            drop(future);
            assert!(owner.journal.pending.is_none());
            assert!(
                owner.journal.operation.is_some(),
                "completed row is not operation commit"
            );
            assert!(owner.socket_generation().await.is_err());
            if kind == "mesh" {
                owner.retain_mesh_sink(prefixes[0].prefix).await.unwrap();
                rejected_lookup(4, false).await;
                rejected_lookup(4, true).await;
            } else {
                owner.capture(&prefixes).await.unwrap(); // resume + real idempotence
                rejected_lookup(4, false).await;
                rejected_lookup(6, false).await;
            }
            assert!(owner.journal.operation.is_none());
            owner.verify().await.unwrap();
            println!(
                "cancel/{kind}/{after}: full intent retained, capability refused, exact owned completion restored"
            );
            owner.relinquish().await.unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "separate process intentionally exits between committed resource rows"]
async fn operation_crash_stage() {
    let directory = Path::new("/run/underlay-operation-state");
    state(directory);
    let kind = std::env::var("MESHWAY_UNDERLAY_TEST_OPERATION").unwrap();
    let mut owner = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    owner.checkpoint = crash_checkpoint(&kind);
    match kind.as_str() {
        "mesh" => owner
            .retain_mesh_sink(capture_prefixes()[0].prefix)
            .await
            .unwrap(),
        "capture" => owner.capture(&capture_prefixes()).await.unwrap(),
        "reservation" => {}
        _ => panic!("invalid fixture operation"),
    }
    panic!("resource boundary did not exit the process");
}

#[tokio::test]
#[ignore = "separate process following operation_crash_stage in the same netns"]
async fn operation_crash_recover() {
    let directory = Path::new("/run/underlay-operation-state");
    let owner = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    assert!(owner.journal.operation.is_none());
    match std::env::var("MESHWAY_UNDERLAY_TEST_RECOVER_KIND")
        .unwrap()
        .as_str()
    {
        "mesh" => {
            rejected_lookup(4, false).await;
            rejected_lookup(4, true).await;
        }
        "capture" => {
            rejected_lookup(4, false).await;
            rejected_lookup(6, false).await;
        }
        "reservation" => owner.verify().await.unwrap(),
        _ => panic!("invalid fixture kind"),
    }
    let generation = owner.socket_generation().await.unwrap();
    generation.retire();
    drop(generation);
    owner.relinquish().await.unwrap();
    println!("separate-process resource-boundary recovery: complete dual-stack topology only");
}

#[tokio::test]
#[ignore = "holder in one mount namespace; contender has private /run"]
async fn singleton_hold() {
    let directory = Path::new("/run/underlay-singleton-state");
    state(directory);
    let owner = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    let generation = owner.socket_generation().await.unwrap();
    let factory = generation.factory();
    generation.retire();
    drop(generation);
    drop(owner);
    std::fs::write(
        "/run/underlay-singleton-ready",
        b"retired factory still owns singleton",
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(60), async {
        while !Path::new("/run/underlay-singleton-release").exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    drop(factory);
    LinuxUnderlay::prepare(binding(directory))
        .await
        .unwrap()
        .relinquish()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "same real netns as singleton_hold but independent mount/state view"]
async fn singleton_contender() {
    let directory = Path::new("/run/underlay-singleton-other-state");
    state(directory);
    let error = match LinuxUnderlay::prepare(binding(directory)).await {
        Ok(_) => panic!("private mount bypassed namespace singleton"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    assert!(!directory.join("underlay.json").exists());
    println!(
        "same netns / independent private run: abstract singleton rejected before kernel mutation"
    );
}

#[tokio::test]
#[ignore = "owned persistent fixture volume, first namespace"]
async fn scope_create() {
    let directory = Path::new("/state/private");
    state(directory);
    std::fs::write(
        directory.join("node-identity"),
        b"preserved-node-credential-fixture",
    )
    .unwrap();
    std::fs::write(
        directory.join("saved-config"),
        b"preserved-network-config-fixture",
    )
    .unwrap();
    let mut owner = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    owner
        .retain_mesh_sink("198.18.76.0/24".parse().unwrap())
        .await
        .unwrap();
    let _tun = crate::managed_dataplane::tun::Tun::open_with_underlay(
        &crate::managed_dataplane::TunOptions {
            name: "mwoldscope0".into(),
            mtu: 1000,
        },
        &tun_recovery_tests::plan(),
        &mut owner,
    )
    .await
    .unwrap();
    assert!(
        owner
            .journal
            .installed
            .iter()
            .any(|row| matches!(row.resource, Resource::Tun { .. }))
    );
    std::process::exit(0);
}

#[tokio::test]
#[ignore = "same owned persistent volume, new namespace after stop/start"]
async fn scope_fresh() {
    let directory = Path::new("/state/private");
    let mut previous: Journal =
        serde_json::from_slice(&std::fs::read(directory.join("underlay.json")).unwrap()).unwrap();
    let (current, _) = current_namespace(&UnixDatagram::unbound().unwrap()).unwrap();
    assert!(!previous.namespace.same_lifetime(&current).unwrap());
    let old_cookie = previous.namespace.cookie;
    // Explicit inode/dev-reuse model against two actual different namespaces:
    // only stored locator metadata is forced; the old kernel cookie is genuine.
    previous.namespace.device = current.device;
    previous.namespace.inode = current.inode;
    write_journal(directory, &previous).unwrap();
    let mut owner = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    assert_eq!(owner.journal.namespace.device, previous.namespace.device);
    assert_eq!(owner.journal.namespace.inode, previous.namespace.inode);
    assert_ne!(owner.journal.namespace.cookie, old_cookie);
    println!(
        "forced same dev/inode: old cookie {:?}, new cookie {:?}; archive/fresh, never old evidence adoption",
        old_cookie, owner.journal.namespace.cookie
    );
    assert_ne!(previous.namespace, owner.journal.namespace);
    assert!(
        owner
            .journal
            .installed
            .iter()
            .all(|item| matches!(item.resource, Resource::Guard { .. } | Resource::Nop { .. }))
    );
    let archives: Vec<_> = std::fs::read_dir(directory.join("underlay-scopes"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(archives.len(), 1);
    let archived: Journal = serde_json::from_slice(&std::fs::read(&archives[0]).unwrap()).unwrap();
    assert_eq!(archived.namespace, previous.namespace);
    assert!(
        archived
            .installed
            .iter()
            .any(|row| matches!(row.resource, Resource::Tun { .. }))
    );
    assert!(!Path::new("/sys/class/net/mwoldscope0").exists());
    let plan = tun_recovery_tests::plan();
    owner.retain_mesh_sink(plan.prefix.into()).await.unwrap();
    tun_recovery_tests::reject_mesh().await;
    tun_recovery_tests::control(&owner).await;
    tun_recovery_tests::fresh(&mut owner, &plan).await;
    assert!(!Path::new("/sys/class/net/mwoldscope0").exists());
    assert_eq!(
        std::fs::read(directory.join("node-identity")).unwrap(),
        b"preserved-node-credential-fixture"
    );
    assert_eq!(
        std::fs::read(directory.join("saved-config")).unwrap(),
        b"preserved-network-config-fixture"
    );
    owner.relinquish().await.unwrap();
    println!(
        "new netns: old scope durably archived, fresh resources allocated, identity/config bytes preserved"
    );
    // Metadata simulation only: host reboot is prohibited by the fixture. All
    // known kernel resources have already been relinquished before this check.
    let mut previous_boot = archived.clone();
    previous_boot.namespace.boot = "simulated-prior-boot".into();
    write_journal(directory, &previous_boot).unwrap();
    let fresh = LinuxUnderlay::prepare(binding(directory)).await.unwrap();
    assert_ne!(fresh.journal.namespace.boot, "simulated-prior-boot");
    assert_eq!(
        std::fs::read_dir(directory.join("underlay-scopes"))
            .unwrap()
            .count(),
        2
    );
    assert_eq!(
        std::fs::read(directory.join("node-identity")).unwrap(),
        b"preserved-node-credential-fixture"
    );
    fresh.relinquish().await.unwrap();
    println!(
        "prior-boot metadata branch: independent archive/fresh allocation, not a real host reboot"
    );
}
