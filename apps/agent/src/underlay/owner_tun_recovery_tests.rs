//! R1: normal nonpersistent TUN open/cancel/restart, not a tun:None matrix.
use super::*;
use crate::managed_dataplane::{PacketPlan, TunOptions, tun::Tun};

fn preparation(path: &Path) -> Preparation<'_> {
    Preparation {
        state_directory: path,
        identity: "tun-recovery-node",
        network: "tun-recovery-network",
    }
}

pub(super) fn plan() -> PacketPlan {
    let (payloads, _) = crate::managed_dataplane::tests::fixtures(2);
    PacketPlan::compile(&payloads[0], 1000).unwrap()
}

pub(super) async fn reject_mesh() {
    for forwarded in [false, true] {
        let mut args = vec![
            "-4".into(),
            "route".into(),
            "get".into(),
            "100.64.0.2".into(),
        ];
        if forwarded {
            args.extend([
                "from".into(),
                "172.30.247.21".into(),
                "iif".into(),
                "eth0".into(),
            ]);
        }
        assert!(ip(args).await.is_err());
    }
}

pub(super) async fn control(owner: &LinuxUnderlay) {
    let channel = owner
        .controller_channel(reqwest::Url::parse("http://172.30.247.20:41861").unwrap())
        .await
        .unwrap();
    let error = channel
        .client()
        .with_api_key("fixture-only")
        .enroll_agent(crate::generated::types::EnrollAgentRequest {
            agent_version: "tun-recovery-fixture".into(),
            name: "isolated".into(),
            node_credential: "a".repeat(64),
        })
        .await
        .unwrap_err();
    match error {
        crate::generated::client::ApiOpError::Api(error) => assert_eq!(error.status, 401),
        error => panic!("controller transport did not recover: {error:?}"),
    }
    channel.retire();
    drop(channel);
    // HTTP drivers release their wrappers asynchronously; singleton/state lock
    // must remain held until their actual references disappear.
    tokio::time::timeout(Duration::from_secs(3), async {
        while Arc::strong_count(&owner.lease) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

pub(super) async fn fresh(owner: &mut LinuxUnderlay, plan: &PacketPlan) {
    let tun = Tun::open_with_underlay(
        &TunOptions {
            name: "mwfresh0".into(),
            mtu: 1000,
        },
        plan,
        owner,
    )
    .await
    .unwrap();
    let lookup = ip(vec![
        "-j".into(),
        "-4".into(),
        "route".into(),
        "get".into(),
        "100.64.0.2".into(),
    ])
    .await
    .unwrap();
    assert!(String::from_utf8(lookup).unwrap().contains("mwfresh0"));
    owner.withdraw_tun().await.unwrap();
    drop(tun);
    reject_mesh().await;
}

#[tokio::test]
#[ignore = "real IPv4 TUN, normal open future drops device at the cancellation boundary"]
async fn normal_open_cancel() {
    for (kind, after) in [("open", 1), ("activate", 0)] {
        let directory = tempfile::Builder::new().tempdir_in("/run").unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let plan = plan();
        let mut owner = LinuxUnderlay::prepare(preparation(directory.path()))
            .await
            .unwrap();
        owner.retain_mesh_sink(plan.prefix.into()).await.unwrap();
        if kind == "activate" {
            fresh(&mut owner, &plan).await;
        }
        let checkpoint = Arc::new(InstallCheckpoint {
            after,
            crash: false,
            reached: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        owner.checkpoint = Some(checkpoint.clone());
        let options = TunOptions {
            name: "mwvanish0".into(),
            mtu: 1000,
        };
        let mut future = Box::pin(Tun::open_with_underlay(&options, &plan, &mut owner));
        tokio::select! {
            _ = checkpoint.reached.notified() => {},
            result = &mut future => panic!("open did not pause: {}", result.is_ok()),
            _ = tokio::time::sleep(Duration::from_secs(10)) => panic!("checkpoint timeout"),
        }
        drop(future); // actual AsyncDevice is gone, original binding cannot return
        assert!(!Path::new("/sys/class/net/mwvanish0").exists());
        owner.checkpoint = None;
        owner.recover_operation().await.unwrap();
        assert!(
            owner
                .journal
                .installed
                .iter()
                .all(|installed| !matches!(installed.resource, Resource::Tun { .. }))
        );
        assert!(owner.journal.operation.is_none());
        reject_mesh().await;
        control(&owner).await;
        fresh(&mut owner, &plan).await;
        owner.relinquish().await.unwrap();
        println!(
            "{kind}/{after}: actual vanished TUN -> protected sink-only -> controller request -> fresh binding"
        );
    }
}

#[tokio::test]
#[ignore = "normal open exits after Sink/Tun/family commit or before activate's first Tun"]
async fn normal_open_crash() {
    use std::os::unix::fs::DirBuilderExt;
    let path = Path::new("/run/tun-operation-state");
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => panic!("{error}"),
    }
    let plan = plan();
    let mut owner = LinuxUnderlay::prepare(preparation(path)).await.unwrap();
    owner.retain_mesh_sink(plan.prefix.into()).await.unwrap();
    let kind = std::env::var("MESHWAY_UNDERLAY_TEST_OPERATION").unwrap();
    if kind == "activate" {
        fresh(&mut owner, &plan).await;
    }
    owner.checkpoint = crash_checkpoint(&kind);
    Tun::open_with_underlay(
        &TunOptions {
            name: "mwvanish0".into(),
            mtu: 1000,
        },
        &plan,
        &mut owner,
    )
    .await
    .unwrap();
    panic!("normal open did not exit at its selected committed-resource boundary");
}

#[tokio::test]
#[ignore = "new process in same namespace after normal_open_crash"]
async fn normal_open_recover() {
    assert!(!Path::new("/sys/class/net/mwvanish0").exists());
    let plan = plan();
    let mut owner = LinuxUnderlay::prepare(preparation(Path::new("/run/tun-operation-state")))
        .await
        .unwrap();
    assert!(
        owner
            .journal
            .installed
            .iter()
            .all(|installed| !matches!(installed.resource, Resource::Tun { .. }))
    );
    assert!(owner.journal.operation.is_none());
    reject_mesh().await;
    control(&owner).await;
    fresh(&mut owner, &plan).await;
    owner.relinquish().await.unwrap();
    println!(
        "same-scope new process: sink-only complete, old TUN absent, controller usable, new binding activated"
    );
}
