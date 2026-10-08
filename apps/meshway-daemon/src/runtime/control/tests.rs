use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN and NET_ADMIN"]
async fn live_tun_owner_conflict_retries_and_recovers_without_new_up() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let fixture = crate::runtime::ping::integration::Fixture::open().await;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let owner_directory = Arc::new(
        meshway_private_storage::Directory::open(&root.join("owner"), "owner.lock").unwrap(),
    );
    let options = meshway_underlay::Options {
        bootstrap_nameservers: Some(vec!["192.0.2.53:53".parse().unwrap()]),
        ..Default::default()
    };
    let mut owner = meshway_underlay::Host::open(owner_directory.clone(), options.clone())
        .await
        .unwrap();
    owner
        .install(fixture.controls[0].execution_snapshot().unwrap())
        .await
        .unwrap();
    let tun_index = owner.tun().unwrap().if_index().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&root.join("restart"), "restart.lock").unwrap(),
    );
    // The restarting process has the old journal while its previous owner is
    // still alive. It must not adopt/delete that owner's TUN or positive routes.
    directory
        .replace(
            "underlay.json",
            &std::fs::read(owner_directory.path().join("underlay.json")).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        meshway_underlay::Host::open(directory.clone(), options.clone()).await,
        Err(meshway_underlay::Error::Conflict)
    ));
    let revision = meshway_model::Version::MIN;
    let (desired, desired_rx) = watch::channel(Desired {
        revision,
        up: true,
        shutdown: false,
        retry_enrollment: false,
        enrollment_key: None,
        rotation: None,
    });
    let (reported, mut report) = watch::channel(Report::initial(revision));
    let (network_reported, _) = watch::channel(network::Report::stopped());
    let (_pings, pings) = tokio::sync::mpsc::channel(1);
    let (events, _) = watch::channel(events::EventStatus::default());
    let mut settings = Settings::new(
        directory.path().to_owned(),
        "http://127.0.0.1:9",
        Some("restart"),
    )
    .unwrap();
    settings.underlay = options;
    let settings = Arc::new(settings);
    let driver = tokio::spawn(run(
        settings,
        desired_rx,
        reported,
        directory,
        network_reported,
        pings,
        events,
    ));
    let first = tokio::time::timeout(
        Duration::from_secs(10),
        report.wait_for(|r| r.problem == Some(Problem::Underlay)),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(first.phase, Phase::Retrying);
    assert!(first.retry_at.is_some());
    let second = tokio::time::timeout(
        Duration::from_secs(5),
        report.wait_for(|r| r.retry_at > first.retry_at),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(second.problem, Some(Problem::Underlay));
    assert_eq!(owner.tun().unwrap().if_index().unwrap(), tun_index);
    owner.shutdown(true).await.unwrap();
    // The same control task gets through bootstrap after the owner releases;
    // unreachable HTTP is now the error, without any operator revision change.
    let recovered = tokio::time::timeout(
        Duration::from_secs(10),
        report.wait_for(|r| r.problem == Some(Problem::Network)),
    )
    .await
    .unwrap()
    .unwrap()
    .clone();
    assert_eq!(recovered.revision, revision);
    assert!(recovered.retry_at.is_some());
    desired.send_modify(|d| {
        d.up = false;
        d.shutdown = true;
    });
    driver.await.unwrap().unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn cancelled_queued_demand_does_not_fence_an_unrelated_live_instance() {
    use crate::runtime::ping::integration::Fixture;
    let mut fixture = Fixture::open().await;
    let control = &mut fixture.controls[0];
    let guard = control.execution_snapshot().unwrap();
    let instance = guard.instance();
    let (reply, receiver) = tokio::sync::oneshot::channel();
    drop(receiver);
    network::request(
        control,
        network::Request::Demand {
            operation: network::Operation::Renew {
                request: meshway_protocol::RenewDemandRequest {
                    instance: instance.clone(),
                    // There is no such demand. Dispatching this abandoned request
                    // would produce a real binding rejection and fence Control.
                    request_id: "00000000-0000-4000-8000-000000000099".parse().unwrap(),
                    expires_at_ms: guard.expires_at_ms(),
                },
                prepared: true,
            },
            reply,
        },
    )
    .await
    .unwrap();
    guard.ensure_current().unwrap();
    assert_eq!(control.execution_snapshot().unwrap().instance(), instance);
    fixture.close().await;
}

#[tokio::test]
async fn ready_report_settles_receipt_without_refreshing_acquisition_authority() {
    use crate::runtime::{network::testing::Worker, ping::integration::Fixture};
    let mut fixture = Fixture::open().await;
    let mut worker = Worker::new();
    let ticket = fixture.controls[0].begin_application().await.unwrap();
    let guard = fixture.controls[0].execution_snapshot().unwrap();
    worker.complete(ticket, guard.clone());
    assert!(worker.handle.diagnostics_ready(&fixture.controls[0]));
    assert!(!worker.handle.can_report_inline(&fixture.controls[0]));
    settle_report(&mut worker.handle, &mut fixture.controls[0])
        .await
        .unwrap();
    assert!(worker.handle.can_report_inline(&fixture.controls[0]));
    guard.ensure_current().unwrap();
    worker.no_commands();
    worker.close().await;
    fixture.close().await;
}

#[tokio::test]
async fn receipt_http_conflicts_preserve_installation_until_binding_requires_suspend() {
    use crate::runtime::{network::testing::Worker, ping::integration::Fixture};
    for (code, refresh) in [
        ("receipt_publication_superseded", true),
        ("receipt_lease_expired", true),
        ("receipt_precondition", true),
        ("receipt_content_conflict", true),
        ("receipt_instance_mismatch", false),
        ("receipt_target_mismatch", false),
    ] {
        let mut fixture = Fixture::open().await;
        let mut worker = Worker::new();
        let ticket = fixture.controls[0].begin_application().await.unwrap();
        let guard = fixture.controls[0].execution_snapshot().unwrap();
        worker.complete(ticket.clone(), guard.clone());
        fixture.reject_receipt(code);
        let error = worker
            .handle
            .finish(&mut fixture.controls[0])
            .await
            .unwrap_err();
        let failure = failure::at("finish_application", error, false);
        assert_eq!(failure.reset, !refresh, "{code}");
        assert_eq!(failure.stops_network(), !refresh, "{code}");
        if failure.stops_network() {
            worker.suspend().await;
            assert!(fixture.controls[0].status().instance.is_none());
            assert!(guard.ensure_current().is_err());
        } else {
            worker.no_commands();
            worker.receipt_retired();
            guard.ensure_current().unwrap();
            worker
                .handle
                .finish(&mut fixture.controls[0])
                .await
                .unwrap();
            fixture.controls[0].refresh_configuration().await.unwrap();
            worker
                .handle
                .publish(&mut fixture.controls[0])
                .await
                .unwrap();
            worker.new_attempt(&ticket);
            worker.no_commands();
        }
        worker.close().await;
        fixture.close().await;
    }
}

#[test]
fn cancelled_private_write_is_settled_and_network_join_runs_before_the_result() {
    // Queue the real Control write behind one blocking worker. No replacement
    // storage implementation or production visibility is needed to cancel its
    // original waiter before the write has a final result.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    for fail in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("identity");
        runtime.block_on(async {
            let mut control = Control::open(
                &path,
                "http://127.0.0.1:9",
                meshway_control_client::http_client_builder()
                    .unwrap()
                    .no_proxy(),
            )
            .unwrap();
            let (entered, entry) = tokio::sync::oneshot::channel();
            let (release, waiting) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                let _ = entered.send(());
                let _ = waiting.recv();
            });
            entry.await.unwrap();
            let mut begin = Box::pin(control.begin_enrollment("shutdown-node"));
            assert!(matches!(
                begin.as_mut().poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            drop(begin);
            assert!(control.status().reload_required);
            if fail {
                // Fail the accepted write's real ownership check, including
                // when running as root (a chmod-based failure would not work).
                std::fs::write(path.join("node-control.lock"), b"foreign marker\n").unwrap();
            }
            release.send(()).unwrap();
            blocker.await.unwrap();

            let (cleaning, started) = tokio::sync::oneshot::channel();
            let (finish, finished) = tokio::sync::oneshot::channel();
            let cleanup = async {
                cleaning.send(()).unwrap();
                // Model the asynchronous join boundary, not network behavior.
                let worker = tokio::spawn(async move { finished.await.unwrap() });
                worker.await.unwrap();
                Ok(())
            };
            let stopping = shutdown(Some(control), cleanup, None);
            tokio::pin!(stopping);
            tokio::select! {
                result = &mut stopping => panic!("returned before network join: {result:?}"),
                result = started => result.unwrap(),
            }
            finish.send(()).unwrap();
            let result = stopping.await;
            assert_eq!(result.is_err(), fail);
            if fail {
                assert_eq!(result, Err("Control private shutdown failed."));
                assert_eq!(
                    std::fs::read(path.join("node-control.lock")).unwrap(),
                    b"foreign marker\n"
                );
            } else {
                let restored = Control::open(
                    &path,
                    "http://127.0.0.1:9",
                    meshway_control_client::http_client_builder()
                        .unwrap()
                        .no_proxy(),
                )
                .unwrap();
                assert!(restored.status().enrollment_request_id.is_some());
                restored.shutdown().await.unwrap();
            }
            // The cancelled write's directory lock is released only after join.
            meshway_private_storage::Directory::open(&path, "node-control.lock").unwrap();
        });
    }
}

#[tokio::test]
async fn prior_settlement_failure_survives_network_cleanup_failure() {
    let result = shutdown(
        None,
        async { Err(Problem::Underlay) },
        Some("prior write failed"),
    )
    .await;
    assert_eq!(result, Err("prior write failed"));
    assert_eq!(
        shutdown(None, async { Err(Problem::Underlay) }, None).await,
        Err("Network shutdown failed.")
    );
}
