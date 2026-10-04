use super::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

#[test]
fn cancelled_durable_write_retains_the_directory_and_requires_reopen() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("private");
    runtime.block_on(async {
        let mut control = Control::open(
            &path,
            "http://127.0.0.1:9",
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap();
        let (entered, entry) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            waiting.recv().unwrap();
        });
        entry.recv().unwrap();
        let mut begin = Box::pin(control.begin_enrollment("durable-node"));
        assert!(matches!(
            Pin::as_mut(&mut begin).poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        drop(begin);
        assert!(control.status().reload_required);
        assert!(control.status().binding.is_none());
        assert!(matches!(
            control.authenticate().await,
            Err(Error::ReloadRequired)
        ));
        assert!(matches!(
            meshway_private_storage::Directory::open(&path, "node-control.lock"),
            Err(meshway_private_storage::Error::InUse)
        ));
        release.send(()).unwrap();
        blocker.await.unwrap();
        control.shutdown().await.unwrap();
        let restored = Control::open(
            &path,
            "http://127.0.0.1:9",
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap();
        assert!(!restored.status().reload_required);
        assert!(restored.status().enrollment_request_id.is_some());
        assert!(!restored.status().authenticated && restored.status().instance.is_none());
        restored.shutdown().await.unwrap();
    });
}
