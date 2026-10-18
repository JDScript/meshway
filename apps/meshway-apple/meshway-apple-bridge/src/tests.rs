use super::*;
use meshway_protocol::ipc::{self, Command, DataPlane, Request, Response, ResultBody};

struct Logs;
impl LogSink for Logs {
    fn line(&self, _: String) {}
}
struct Frames(tokio::sync::mpsc::Sender<Vec<u8>>);
impl FrameSink for Frames {
    fn on_frame(&self, frame: Vec<u8>) -> bool {
        self.0.try_send(frame).is_ok()
    }
}

#[test]
fn real_node_status_and_joined_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let config = StartConfig {
        state_directory: directory
            .path()
            .canonicalize()
            .unwrap()
            .join("node")
            .to_str()
            .unwrap()
            .into(),
        server_origin: "http://127.0.0.1:8430".into(),
        hostname: Some("apple-bridge-test".into()),
        dns_anchor_address: None,
    };
    let node = start_control_only(config, Box::new(Logs)).unwrap();
    runtime().unwrap().block_on(async {
        let (frames, mut received) = tokio::sync::mpsc::channel(1);
        let session = node.open_ipc_session(Box::new(Frames(frames))).unwrap();
        assert!(matches!(
            session.send(vec![0, 0, 0, 1]),
            Err(BridgeError::Frame)
        ));
        let request = Request {
            version: ipc::VERSION,
            request_id: "552281b5-ae2b-4fad-9ce5-8b60c2b48a7f".parse().unwrap(),
            command: Command::Status,
        };
        let mut bytes = Vec::new();
        ipc::write(&mut bytes, &request).await.unwrap();
        session.send(bytes.clone()).unwrap();
        assert!(matches!(session.send(bytes), Err(BridgeError::Frame)));
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
        let response: Response = ipc::read(&mut frame.as_slice()).await.unwrap();
        assert_eq!(response.version, 13);
        assert_eq!(response.request_id, request.request_id);
        let ResultBody::Status { status } = response.result else {
            panic!("real status required")
        };
        assert_eq!(status.data_plane, DataPlane::ControlOnly);
        assert!(status.applied.is_none());
        session.close().await;
        let mut held = Vec::new();
        for _ in 0..8 {
            let (send, _) = tokio::sync::mpsc::channel(1);
            held.push(node.open_ipc_session(Box::new(Frames(send))).unwrap());
        }
        let (send, _) = tokio::sync::mpsc::channel(1);
        assert!(matches!(
            node.open_ipc_session(Box::new(Frames(send))),
            Err(BridgeError::Capacity)
        ));
        // Hold completion after the real Node exits. Cancelling one FFI stop
        // waiter must not detach the join or let another waiter finish early.
        let entered = Arc::new(tokio::sync::Notify::new());
        let reached = entered.clone();
        let (release, released) = tokio::sync::oneshot::channel();
        let original = node.task.lock().await.take().unwrap();
        *node.task.lock().await = Some(tokio::spawn(async move {
            let result = original.await.unwrap();
            reached.notify_one();
            let _ = released.await;
            result
        }));
        {
            let stopping = node.stop();
            tokio::pin!(stopping);
            tokio::select! {
                _ = entered.notified() => {},
                result = &mut stopping => panic!("stop finished before join: {result:?}"),
            }
        }
        let stopping = node.stop();
        tokio::pin!(stopping);
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(stopping.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        release.send(()).unwrap();
        stopping.await.unwrap();
        assert!(held.iter().all(|session| session.is_closed()));
        node.stop().await.unwrap();
    });
}
