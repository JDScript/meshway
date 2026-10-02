//! Bootstrap rejection and handoff ownership, using actual kernel resources.
use super::*;

#[tokio::test]
#[ignore = "requires disposable Linux namespace with NET_ADMIN and audit tools"]
async fn guard_only_capability_requires_retirement_and_preserves_foreign_resources() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_FIXTURE").as_deref(),
        Ok("1")
    );
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let fingerprint = "a".repeat(64);
    let prepare = || BootstrapPreparation {
        state_directory: directory.path(),
        fingerprint: &fingerprint,
    };
    let case = std::env::var("MESHWAY_UNDERLAY_CASE").unwrap_or_else(|_| "capability".into());
    if case == "foreign" {
        ip(vec![
            "-4",
            "route",
            "add",
            "unreachable",
            "100.96.0.0/11",
            "proto",
            "242",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect())
        .await
        .unwrap();
        let before = objects(4, false).await.unwrap();
        assert!(BootstrapUnderlay::prepare(prepare()).await.is_err());
        assert_eq!(objects(4, false).await.unwrap(), before);
        assert!(!directory.path().join("bootstrap.json").exists());
        return;
    }
    if case == "orphan-capture" {
        ip(vec![
            "-4", "rule", "add", "priority", "18001", "iif", "lo", "lookup", "40040",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect())
        .await
        .unwrap();
        let before = objects(4, true).await.unwrap();
        assert!(BootstrapUnderlay::prepare(prepare()).await.is_err());
        assert_eq!(objects(4, true).await.unwrap(), before);
        return;
    }
    if case == "foreign-tun" {
        let device = tun_rs::DeviceBuilder::new()
            .enable(false)
            .build_async()
            .unwrap();
        let name = device.name().unwrap();
        assert!(BootstrapUnderlay::prepare(prepare()).await.is_err());
        assert!(Path::new("/sys/class/net").join(name).exists());
        return;
    }
    if case == "bound-journal" {
        let file = directory.path().join("underlay.json");
        std::fs::write(&file, b"{\"version\":3}").unwrap();
        assert!(BootstrapUnderlay::prepare(prepare()).await.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"{\"version\":3}");
        return;
    }
    let mut owner = BootstrapUnderlay::prepare(prepare()).await.unwrap();
    assert_eq!(owner.core.journal.installed.len(), 4);
    if case == "cleanup" {
        owner.core.journal.cleaning = true;
        write_journal(directory.path(), &owner.core.journal).unwrap();
        owner.core.remove_one(3).await.unwrap();
        drop(owner); // interruption after an exactly evidenced deletion
        let owner = BootstrapUnderlay::prepare(prepare()).await.unwrap();
        assert!(
            owner
                .channel(reqwest::Url::parse("http://127.0.0.1:8420").unwrap())
                .await
                .is_err()
        );
        let bound = owner
            .bind(Preparation {
                state_directory: directory.path(),
                identity: "fixture-node",
                network: "fixture-network",
            })
            .await
            .unwrap();
        assert!(!directory.path().join("bootstrap.json").exists());
        assert_eq!(bound.journal.version, 3);
        return;
    }
    if case == "lifetime" || case == "damage" {
        let file = directory.path().join("bootstrap.json");
        drop(owner);
        let mut journal: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        if case == "lifetime" {
            journal["namespace"]
                .as_object_mut()
                .unwrap()
                .remove("cookie");
            std::fs::write(&file, serde_json::to_vec(&journal).unwrap()).unwrap();
        } else {
            std::fs::write(&file, b"damaged-journal").unwrap();
        }
        let before_rules = objects(4, true).await.unwrap();
        let before = std::fs::read(&file).unwrap();
        assert!(BootstrapUnderlay::prepare(prepare()).await.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert_eq!(objects(4, true).await.unwrap(), before_rules);
        return;
    }
    let generation = owner.core.prepare_generation(true).await.unwrap();
    let factory = generation.factory();
    for purpose in [
        super::super::super::Purpose::Carrier,
        super::super::super::Purpose::Stun,
    ] {
        assert!(
            factory
                .udp(purpose, "127.0.0.1:0".parse().unwrap())
                .is_err()
        );
    }
    assert!(
        factory
            .listener(
                super::super::super::Purpose::Controller,
                "127.0.0.1:0".parse().unwrap()
            )
            .is_err()
    );
    assert!(
        owner
            .bind(Preparation {
                state_directory: directory.path(),
                identity: "fixture-node",
                network: "fixture-network"
            })
            .await
            .is_err()
    );
    assert!(BootstrapUnderlay::prepare(prepare()).await.is_err()); // singleton still held by capability
    generation.retire();
    assert!(
        factory
            .udp(
                super::super::super::Purpose::Controller,
                "127.0.0.1:0".parse().unwrap()
            )
            .is_err()
    );
    drop((generation, factory));
    let owner = BootstrapUnderlay::prepare(prepare()).await.unwrap();
    assert_eq!(owner.core.journal.installed.len(), 4);
}
