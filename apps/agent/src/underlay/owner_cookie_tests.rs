//! Actual fixed-dependency getter and Journal v3 negative/lifetime evidence.
use super::*;

#[test]
fn cookie_identity_has_no_inode_or_socket_fallback() {
    assert!(checked_cookie(0).is_err());
    let error = checked_cookie_result(Err(io::Error::from_raw_os_error(92))).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(92));
    let one = Namespace {
        boot: "boot".into(),
        cookie: Some(41),
        device: 1,
        inode: 2,
    };
    let mut other = one.clone();
    other.cookie = Some(42);
    assert!(!one.same_lifetime(&other).unwrap());
    other.cookie = None;
    assert!(one.same_lifetime(&other).is_err());
    other.cookie = Some(41);
    other.inode = 3;
    assert!(one.same_lifetime(&other).unwrap());
    assert!(!one.matching_locator(&other));
    other.boot = "another boot".into();
    assert!(!one.same_lifetime(&other).unwrap());
}

#[tokio::test]
#[ignore = "actual Linux safe wrapper on AF_UNIX and IPv4/IPv6, no bind/connect/send on witness sockets"]
async fn cookie_observe() {
    let socket = UnixDatagram::unbound().unwrap();
    let (namespace, pinned) = current_namespace(&socket).unwrap();
    for _ in 0..2 {
        let other = UnixDatagram::unbound().unwrap();
        assert_eq!(socket_cookie(&other).unwrap(), namespace.cookie.unwrap());
    }
    for domain in [socket2::Domain::IPV4, socket2::Domain::IPV6] {
        let socket =
            socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))
                .unwrap();
        assert_eq!(
            checked_cookie(socket.netns_cookie().unwrap()).unwrap(),
            namespace.cookie.unwrap()
        );
    }
    assert_eq!(pinned.metadata().unwrap().ino(), namespace.inode);
    println!(
        "NETNS_COOKIE_OBSERVATION {}",
        serde_json::to_string(&namespace).unwrap()
    );
    println!(
        "kernel {}",
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap()
            .trim()
    );
}

#[tokio::test]
#[ignore = "strace injects ENOPROTOOPT into the first parent getsockopt (SO_NETNS_COOKIE)"]
async fn cookie_unsupported() {
    let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let before4 = objects(4, true).await.unwrap();
    let before6 = objects(6, true).await.unwrap();
    let before_routes4 = objects(4, false).await.unwrap();
    let before_routes6 = objects(6, false).await.unwrap();
    let error = match LinuxUnderlay::prepare(Preparation {
        state_directory: directory.path(),
        identity: "cookie-failure",
        network: "cookie-network",
    })
    .await
    {
        Ok(_) => panic!("unsupported lifetime getter published owner"),
        Err(error) => error,
    };
    assert_eq!(error.raw_os_error(), Some(92));
    assert!(!directory.path().join("underlay.json").exists());
    assert_eq!(before4, objects(4, true).await.unwrap());
    assert_eq!(before6, objects(6, true).await.unwrap());
    assert_eq!(before_routes4, objects(4, false).await.unwrap());
    assert_eq!(before_routes6, objects(6, false).await.unwrap());
    println!(
        "actual injected ENOPROTOOPT: original error preserved, no journal/RPDB/route mutation"
    );
}

#[tokio::test]
#[ignore = "private malformed/legacy journals stay intact and cannot authorize kernel resources"]
async fn legacy_and_damage_refused() {
    let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let input = || Preparation {
        state_directory: directory.path(),
        identity: "cookie-node",
        network: "cookie-network",
    };
    let owner = LinuxUnderlay::prepare(input()).await.unwrap();
    let journal = owner.journal.clone();
    drop(owner);
    let before4 = objects(4, true).await.unwrap();
    let before6 = objects(6, true).await.unwrap();
    for variant in 0..6 {
        let mut candidate = journal.clone();
        match variant {
            0 => {
                candidate.version = 2;
                candidate.namespace.cookie = None;
            }
            1 => candidate.namespace.cookie = None,
            2 => candidate.namespace.cookie = Some(0),
            3 => candidate.identity = "another-node".into(),
            4 => candidate.network = "another-network".into(),
            _ => candidate.namespace.inode += 1, // same cookie, inconsistent locator
        }
        write_journal(directory.path(), &candidate).unwrap();
        if variant == 0 {
            // Exact old wire shape: cookie field absent, not merely null.
            let mut old = serde_json::to_value(&candidate).unwrap();
            old["namespace"].as_object_mut().unwrap().remove("cookie");
            std::fs::write(
                directory.path().join("underlay.json"),
                serde_json::to_vec(&old).unwrap(),
            )
            .unwrap();
        }
        let bytes = std::fs::read(directory.path().join("underlay.json")).unwrap();
        assert!(LinuxUnderlay::prepare(input()).await.is_err());
        assert_eq!(
            bytes,
            std::fs::read(directory.path().join("underlay.json")).unwrap()
        );
        assert!(!directory.path().join("underlay-scopes").exists());
        assert_eq!(before4, objects(4, true).await.unwrap());
        assert_eq!(before6, objects(6, true).await.unwrap());
    }
    write_journal(directory.path(), &journal).unwrap();
    // Same lifetime but missing evidence is damage, NOT a new-scope signal.
    let resource = journal.installed[0].resource.clone();
    ip(resource.args("del")).await.unwrap();
    let bytes = std::fs::read(directory.path().join("underlay.json")).unwrap();
    assert!(LinuxUnderlay::prepare(input()).await.is_err());
    assert_eq!(
        bytes,
        std::fs::read(directory.path().join("underlay.json")).unwrap()
    );
    assert!(!directory.path().join("underlay-scopes").exists());
    // This fixture itself deleted this exact owned rule; restore it explicitly.
    ip(resource.args("add")).await.unwrap();
    LinuxUnderlay::prepare(input())
        .await
        .unwrap()
        .relinquish()
        .await
        .unwrap();
    println!(
        "legacy/missing/zero/binding/locator journals and same-cookie damage refused without adopting or discarding evidence"
    );
}

#[tokio::test]
#[ignore = "scope archive/fresh durable checkpoints, simulated prior boot only; no host reboot"]
async fn scope_crash_stage() {
    use std::os::unix::fs::DirBuilderExt;
    let boundary = std::env::var("MESHWAY_UNDERLAY_SCOPE_BOUNDARY").unwrap();
    let directory = PathBuf::from(format!("/run/cookie-scope-{boundary}"));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    std::fs::write(directory.join("node-identity"), b"preserve identity").unwrap();
    std::fs::write(directory.join("saved-config"), b"preserve config").unwrap();
    let input = || Preparation {
        state_directory: &directory,
        identity: "scope-crash-node",
        network: "scope-crash-network",
    };
    let owner = LinuxUnderlay::prepare(input()).await.unwrap();
    let mut journal = owner.journal.clone();
    owner.relinquish().await.unwrap();
    journal.namespace.boot = "simulated-prior-boot".into();
    write_journal(&directory, &journal).unwrap();
    // Test-process-only env set by runner: it arms only this second prepare.
    // No unsafe set_var (Rust 2024): checkpoint uses separate boundary env plus
    // the stage's private marker. fresh-journal is before reservation operation.
    std::fs::write(directory.join("scope-exit-armed"), b"test checkpoint").unwrap();
    LinuxUnderlay::prepare(input()).await.unwrap();
    panic!("scope crash checkpoint did not exit");
}

#[tokio::test]
#[ignore = "new process after durable archive/fresh-journal; preserve old scope bytes and normal recovery"]
async fn scope_crash_recover() {
    let boundary = std::env::var("MESHWAY_UNDERLAY_SCOPE_BOUNDARY").unwrap();
    let directory = PathBuf::from(format!("/run/cookie-scope-{boundary}"));
    let archives: Vec<_> = std::fs::read_dir(directory.join("underlay-scopes"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(archives.len(), 1);
    let bytes = std::fs::read(&archives[0]).unwrap();
    let archived: Journal = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(archived.namespace.boot, "simulated-prior-boot");
    assert_eq!(
        std::fs::metadata(&archives[0]).unwrap().mode() & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(directory.join("underlay-scopes"))
            .unwrap()
            .mode()
            & 0o777,
        0o700
    );
    let input = || Preparation {
        state_directory: &directory,
        identity: "scope-crash-node",
        network: "scope-crash-network",
    };
    let recovered = LinuxUnderlay::prepare(input()).await;
    let owner = if boundary == "fresh-journal" {
        let error = match recovered {
            Ok(_) => panic!("partial reservation must report rollback/retry"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(!directory.join("underlay.json").exists());
        println!(
            "fresh-journal: existing partial reservation contract reported exact rollback/retry"
        );
        LinuxUnderlay::prepare(input()).await.unwrap()
    } else {
        recovered.unwrap()
    };
    owner.verify().await.unwrap();
    assert_eq!(bytes, std::fs::read(&archives[0]).unwrap());
    assert_eq!(
        std::fs::read(directory.join("node-identity")).unwrap(),
        b"preserve identity"
    );
    assert_eq!(
        std::fs::read(directory.join("saved-config")).unwrap(),
        b"preserve config"
    );
    owner.relinquish().await.unwrap();
    println!(
        "{boundary}: simulated prior-boot process-exit recovered; immutable private archive/identity/config preserved"
    );
}

#[tokio::test]
#[ignore = "different lifetime metadata cannot bypass foreign-selector new-scope preflight"]
async fn rollover_guard_refused() {
    let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let input = || Preparation {
        state_directory: directory.path(),
        identity: "guard-node",
        network: "guard-network",
    };
    let owner = LinuxUnderlay::prepare(input()).await.unwrap();
    let original = owner.journal.clone();
    drop(owner);
    let mut candidate = original.clone();
    candidate.namespace.cookie = Some(candidate.namespace.cookie.unwrap().checked_add(1).unwrap());
    write_journal(directory.path(), &candidate).unwrap();
    let bytes = std::fs::read(directory.path().join("underlay.json")).unwrap();
    let add: Vec<String> = [
        "-4", "rule", "add", "pref", "19002", "uidrange", "1-2", "lookup", "18551",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    ip(add).await.unwrap();
    let before = objects(4, true).await.unwrap();
    assert!(LinuxUnderlay::prepare(input()).await.is_err());
    assert_eq!(before, objects(4, true).await.unwrap());
    assert_eq!(
        bytes,
        std::fs::read(directory.path().join("underlay.json")).unwrap()
    );
    assert!(!directory.path().join("underlay-scopes").exists());
    ip([
        "-4", "rule", "del", "pref", "19002", "uidrange", "1-2", "lookup", "18551",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect())
    .await
    .unwrap();
    write_journal(directory.path(), &original).unwrap();
    LinuxUnderlay::prepare(input())
        .await
        .unwrap()
        .relinquish()
        .await
        .unwrap();
    println!(
        "different-cookie metadata simulation: foreign guard preflight refused, journal/kernel evidence preserved; no archive escape"
    );
}
