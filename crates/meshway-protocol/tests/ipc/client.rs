#![cfg(unix)]
use meshway_protocol::ipc::{
    Client, ClientError, Command, ErrorCode, Request, Response, ResultBody, VERSION,
};
use std::{fs, os::unix::fs::PermissionsExt};
use tokio::net::UnixListener;

#[tokio::test]
async fn client_checks_response_identity_before_exposing_remote_errors() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let server = tokio::spawn(async move {
        for case in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request: Request = meshway_protocol::ipc::read(&mut stream).await.unwrap();
            let response = Response {
                version: if case == 0 { VERSION + 1 } else { VERSION },
                request_id: if case == 1 {
                    "00000000-0000-4000-8000-000000000001".parse().unwrap()
                } else {
                    request.request_id
                },
                result: ResultBody::Error {
                    code: ErrorCode::Conflict,
                },
            };
            meshway_protocol::ipc::write(&mut stream, &response)
                .await
                .unwrap();
        }
    });
    let client = Client::new(&path);
    for _ in 0..2 {
        assert!(matches!(
            client.call(Command::Status).await,
            Err(ClientError::Response)
        ));
    }
    assert!(matches!(
        client.call(Command::Status).await,
        Err(ClientError::Remote(ErrorCode::Conflict))
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn client_rejects_nonprivate_socket_before_sending_a_request() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        Client::new(&path).call(Command::Status).await,
        Err(ClientError::Identity)
    ));
    let (mut stream, _) = listener.accept().await.unwrap();
    use tokio::io::AsyncReadExt;
    assert_eq!(stream.read(&mut [0u8; 1]).await.unwrap(), 0);
}
