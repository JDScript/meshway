use std::{
    future::Future,
    pin::Pin,
    sync::mpsc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use super::*;

#[test]
fn phc_profile_is_validated_before_allocating_argon_memory() {
    bounded_phc(DUMMY).unwrap();
    for value in [
        DUMMY.replace("m=19456", "m=4294967295"),
        DUMMY.replace("t=2", "t=4294967295"),
        DUMMY.replace("p=1", "p=99"),
        DUMMY.replace("v=19", "v=16"),
        DUMMY.replace("argon2id", "argon2i"),
        DUMMY.replace("p=1", "p=1,data=Zm9v"),
        DUMMY.replace("$AAAAAAAAAAAAAAAAAAAAAA$", "$AA$"),
        "x".repeat(257),
    ] {
        assert!(bounded_phc(&value).is_err());
    }
}

#[tokio::test]
async fn cancellation_keeps_budget_until_real_blocking_kdf_finishes() {
    let _serial = TEST_KDF_LOCK.lock().await;
    let other_slots = KDFS.acquire_many((MAX_KDFS - 1) as u32).await.unwrap();
    let (started, running) = tokio::sync::oneshot::channel();
    let (completed, completion) = tokio::sync::oneshot::channel();
    let (release, wait) = mpsc::channel();
    let mut work = Box::pin(blocking(move || {
        started.send(()).unwrap();
        wait.recv().unwrap();
        // Exactly the production algorithm and profile, not a cheap test KDF.
        let result = algorithm()?.verify_password(b"a dummy password", &bounded_phc(DUMMY)?);
        assert!(result.is_err());
        completed.send(()).unwrap();
        Ok(())
    }));
    assert!(matches!(
        Pin::as_mut(&mut work).poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    running.await.unwrap();
    drop(work);
    assert_eq!(KDFS.available_permits(), 0);
    assert!(matches!(
        hash(Secret::new("valid password".into())).await,
        Err(AccountError::Busy)
    ));
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(10), completion)
        .await
        .unwrap()
        .unwrap();
    let finished = tokio::time::timeout(Duration::from_secs(10), KDFS.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(finished);
    drop(other_slots);
    assert_eq!(KDFS.available_permits(), MAX_KDFS);
}
