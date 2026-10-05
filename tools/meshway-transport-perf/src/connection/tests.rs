use super::*;
use std::{
    future::{Future, pending, poll_fn},
    task::Poll,
};

#[tokio::test]
async fn cancelled_join_retains_peer_for_owner_abort_and_cleanup() {
    let (started, ready) = tokio::sync::oneshot::channel();
    let (released, cleanup) = tokio::sync::oneshot::channel();
    struct Resource(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Resource {
        fn drop(&mut self) {
            let _ = self.0.take().unwrap().send(());
        }
    }
    let mut peer = Some(tokio::spawn(async move {
        let _resource = Resource(Some(released));
        started.send(()).unwrap();
        pending::<Result<(), Error>>().await
    }));
    ready.await.unwrap();
    let mut closing = Box::pin(join_peer(&mut peer));
    poll_fn(|cx| {
        assert!(closing.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(closing);
    peer.as_ref()
        .expect("cancelled close retains the owner handle")
        .abort();
    assert!(join_peer(&mut peer).await.is_err());
    cleanup.await.unwrap();
    assert!(peer.is_none());
    join_peer(&mut peer).await.unwrap();
}
