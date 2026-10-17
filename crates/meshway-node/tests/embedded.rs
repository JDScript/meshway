//! Embedding needs no Unix listener or process service manager.
use meshway_node::{Lifecycle, Node, Settings, host::ControlOnly};
use meshway_protocol::ipc::{self, Command, ErrorCode, NetworkMode, Request, Response, ResultBody};

#[tokio::test]
async fn authenticated_memory_streams_share_bounded_dispatch_and_joined_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("node");
    let settings = Settings::new(path, "http://127.0.0.1:9", Some("embedded"))
        .unwrap()
        .with_network_mode(NetworkMode::ControlOnly);
    let node = Node::open(settings.clone(), std::sync::Arc::new(ControlOnly))
        .await
        .unwrap();
    assert!(
        Node::open(settings.clone(), std::sync::Arc::new(ControlOnly))
            .await
            .is_err(),
        "one durable owner"
    );
    let handle = node.ipc();
    let mut lifecycle = node.lifecycle();
    let mut clients = Vec::new();
    for _ in 0..32 {
        let (client, server) = tokio::io::duplex(4096);
        handle.submit(server).unwrap();
        clients.push(client);
    }
    assert_eq!(
        handle.submit(tokio::io::duplex(4096).1),
        Err(ErrorCode::Busy)
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let running = tokio::spawn(node.run(async {
        let _ = stopped.await;
    }));
    lifecycle
        .wait_for(|state| *state == Lifecycle::Ready)
        .await
        .unwrap();
    let request = Request {
        version: ipc::VERSION,
        request_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
        command: Command::Status,
    };
    let client = &mut clients[0];
    ipc::write(client, &request).await.unwrap();
    let response: Response = ipc::read(client).await.unwrap();
    assert_eq!(response.request_id, request.request_id);
    assert!(matches!(response.result, ResultBody::Status { .. }));
    stop.send(()).unwrap();
    running.await.unwrap().unwrap();
    assert_eq!(*lifecycle.borrow(), Lifecycle::Stopped);
    assert_eq!(
        handle.submit(tokio::io::duplex(4096).1),
        Err(ErrorCode::Stopping)
    );
    let reopened = Node::open(settings, std::sync::Arc::new(ControlOnly))
        .await
        .unwrap();
    drop(reopened);
}
