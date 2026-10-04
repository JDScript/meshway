use std::collections::VecDeque;

use meshway_transport::{Frame, Limits, SendContext, SendOutcome, raw::Stream};
use meshway_transport_spec::{SocketOptions, TcpOptions};
use socket2::SockRef;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::TcpTransport;

async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, accepted) = tokio::join!(
        TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept()
    );
    (client.unwrap(), accepted.unwrap().0)
}

fn adopt(stream: TcpStream) -> TcpTransport {
    TcpTransport::adopt(stream, &SocketOptions::default(), &TcpOptions::default()).unwrap()
}

#[tokio::test]
async fn raw_object_safe_stream_uses_adopted_socket_in_both_directions() {
    let (client, server) = pair().await;
    let mut client: Box<dyn Stream> = Box::new(adopt(client));
    let mut server = adopt(server);
    client.write_all(b"request").await.unwrap();
    let mut request = [0; 7];
    server.read_exact(&mut request).await.unwrap();
    assert_eq!(&request, b"request");
    server.write_all(b"reply").await.unwrap();
    let mut reply = [0; 5];
    client.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"reply");
}

#[tokio::test]
async fn native_options_are_applied_and_invalid_values_fail() {
    let (client, _server) = pair().await;
    let buffers = SocketOptions {
        receive_buffer: 16 * 1024,
        send_buffer: 16 * 1024,
    };
    let tcp = TcpOptions::default();
    let client = TcpTransport::adopt(client, &buffers, &tcp)
        .unwrap()
        .into_inner();
    let socket = SockRef::from(&client);
    assert!(client.nodelay().unwrap());
    assert!(socket.keepalive().unwrap());
    assert!(socket.recv_buffer_size().unwrap() >= buffers.receive_buffer as usize);
    assert!(socket.send_buffer_size().unwrap() >= buffers.send_buffer as usize);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        assert_eq!(
            socket.tcp_keepalive_time().unwrap(),
            tcp.keep_alive.idle.as_std()
        );
        assert_eq!(
            socket.tcp_keepalive_interval().unwrap(),
            tcp.keep_alive.interval.as_std()
        );
        assert_eq!(
            socket.tcp_keepalive_retries().unwrap(),
            tcp.keep_alive.count
        );
    }
    let (client, _server) = pair().await;
    let mut invalid = TcpOptions::default();
    invalid.keep_alive.interval = "500ms".parse().unwrap();
    assert_eq!(
        TcpTransport::adopt(client, &buffers, &invalid)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
    let (client, _server) = pair().await;
    let buffers = SocketOptions {
        receive_buffer: u32::MAX,
        send_buffer: 0,
    };
    assert_eq!(
        TcpTransport::adopt(client, &buffers, &tcp)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn real_tcp_decodes_fragmented_and_coalesced_frames() {
    let (client, server) = pair().await;
    let mut raw = adopt(client);
    let mut session = adopt(server)
        .into_session(Limits::new(64, 8, 512, 4, 4).unwrap())
        .unwrap();
    raw.write_all(&[0, 0]).await.unwrap();
    raw.write_all(&[0, 5, b'h', b'e']).await.unwrap();
    raw.write_all(b"llo\0\0\0\x05world").await.unwrap();
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"hello");
    assert_eq!(session.receiver().recv().await.unwrap().payload, b"world");
    session.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_tcp_sessions_send_batches_both_directions() {
    let (client, server) = pair().await;
    let limits = Limits::new(64, 8, 512, 4, 4).unwrap();
    let mut client = adopt(client).into_session(limits).unwrap();
    let mut server = adopt(server).into_session(limits).unwrap();
    let mut outbound = VecDeque::from([
        Frame::new(b"first".to_vec(), None),
        Frame::new(b"second".to_vec(), None),
    ]);
    let sent = client
        .sender()
        .try_send(&mut outbound, SendContext::default());
    assert_eq!(sent.accepted, 2);
    let first = server.receiver().recv().await.unwrap();
    let second = server.receiver().recv().await.unwrap();
    assert_eq!(first.payload, b"first");
    assert_eq!(second.payload, b"second");
    assert!(
        sent.completion
            .unwrap()
            .wait()
            .await
            .unwrap()
            .frames
            .iter()
            .all(|frame| matches!(frame.outcome, SendOutcome::LocalComplete))
    );
    // Stream replies target the same connected peer, not the original
    // receiver's metadata copied blindly into a different endpoint.
    let mut replies = VecDeque::from([
        Frame::new(first.payload, None),
        Frame::new(second.payload, None),
    ]);
    let sent = server
        .sender()
        .try_send(&mut replies, SendContext::default());
    assert_eq!(client.receiver().recv().await.unwrap().payload, b"first");
    assert_eq!(client.receiver().recv().await.unwrap().payload, b"second");
    sent.completion.unwrap().wait().await.unwrap();
    client.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}
