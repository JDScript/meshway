use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

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
