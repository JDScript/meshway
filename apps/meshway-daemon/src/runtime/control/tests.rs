use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

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
                reqwest::Client::builder().no_proxy(),
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
                    reqwest::Client::builder().no_proxy(),
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
