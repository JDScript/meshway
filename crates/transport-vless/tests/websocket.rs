use futures_util::{SinkExt, StreamExt};
use meshway_transport_vless::websocket::{self, Parameters};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{
    Message,
    protocol::{
        Role,
        frame::{
            Frame,
            coding::{Data, OpCode},
        },
    },
};

fn parameters() -> Parameters {
    Parameters {
        path: "/meshway".into(),
        host: "meshway.test".into(),
    }
}

#[tokio::test]
async fn upgrade_headers_have_a_total_byte_budget_and_cancelled_carriers_are_closed() {
    tokio::time::timeout(Duration::from_secs(2), async {
        let (a, mut b) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            assert!(websocket::accept(a, &parameters()).await.is_err());
        });
        let _ = b
            .write_all(&vec![b'A'; websocket::MAX_UPGRADE_BYTES + 1])
            .await;
        drop(b);
        server.await.unwrap();
        let (a, b) = tokio::io::duplex(1024);
        let reader = tokio::spawn(async move {
            let mut carrier = websocket::accept(a, &parameters()).await.unwrap();
            let mut byte = [0];
            assert_eq!(carrier.read(&mut byte).await.unwrap(), 1);
            assert!(carrier.read(&mut byte).await.is_err());
        });
        let mut writer = websocket::connect(b, &parameters()).await.unwrap();
        writer.write_all(b"x").await.unwrap();
        writer.flush().await.unwrap();
        // Dropping a cancelled carrier does not attempt to reuse or reconnect
        // its partial byte stream. The peer sees an abnormal EOF.
        drop(writer);
        reader.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn binary_byte_stream_is_full_duplex_bounded_backpressured_and_closes() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (a, b) = tokio::io::duplex(1024);
        let server = tokio::spawn(async move {
            let mut stream = websocket::accept(a, &parameters()).await.unwrap();
            let mut bytes = vec![0; 512 * 1024];
            stream.read_exact(&mut bytes).await.unwrap();
            assert!(bytes.iter().all(|b| *b == 42));
            stream.write_all(b"reverse").await.unwrap();
            stream.flush().await.unwrap();
            let mut end = [0; 1];
            assert_eq!(stream.read(&mut end).await.unwrap(), 0);
        });
        let mut client = websocket::connect(b, &parameters()).await.unwrap();
        client.write_all(&vec![42; 512 * 1024]).await.unwrap();
        client.flush().await.unwrap();
        let mut reply = [0; 7];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"reverse");
        client.shutdown().await.unwrap();
        server.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn exact_path_and_host_mismatches_reject_http_upgrade() {
    for bad in [
        Parameters {
            path: "/wrong".into(),
            host: "meshway.test".into(),
        },
        Parameters {
            path: "/meshway".into(),
            host: "wrong.test".into(),
        },
    ] {
        bad.validate().unwrap();
        let (a, b) = tokio::io::duplex(8192);
        let expected = parameters();
        let (server, client) =
            tokio::join!(websocket::accept(a, &expected), websocket::connect(b, &bad));
        assert!(server.is_err());
        let error = client.err().expect("upgrade must reject");
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
        assert!(error.to_string().contains("HTTP 404"));
    }
}

#[tokio::test]
async fn fragmented_masked_binary_ping_pong_text_and_size_limits_are_library_enforced() {
    let (a, b) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut stream = websocket::accept(a, &parameters()).await.unwrap();
        let mut bytes = [0; 6];
        stream.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"abcdef");
        assert!(stream.read(&mut bytes).await.is_err());
    });
    let (mut raw, _) = tokio_tungstenite::client_async("wss://meshway.test/meshway", b)
        .await
        .unwrap();
    raw.send(Message::Frame(Frame::message(
        b"abc".to_vec(),
        OpCode::Data(Data::Binary),
        false,
    )))
    .await
    .unwrap();
    raw.send(Message::Ping(b"proof".to_vec().into()))
        .await
        .unwrap();
    raw.send(Message::Frame(Frame::message(
        b"def".to_vec(),
        OpCode::Data(Data::Continue),
        true,
    )))
    .await
    .unwrap();
    assert_eq!(
        raw.next().await.unwrap().unwrap(),
        Message::Pong(b"proof".to_vec().into())
    );
    raw.send(Message::Text("not bytes".into())).await.unwrap();
    server.await.unwrap();
    let (a, b) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut stream = websocket::accept(a, &parameters()).await.unwrap();
        let mut bytes = [0; 1];
        assert!(stream.read(&mut bytes).await.is_err());
    });
    let (mut raw, _) = tokio_tungstenite::client_async("wss://meshway.test/meshway", b)
        .await
        .unwrap();
    let _ = raw
        .send(Message::Binary(
            vec![0; websocket::MAX_MESSAGE_BYTES + 1].into(),
        ))
        .await;
    server.await.unwrap();
    // Server role rejects unmasked client frames, rather than accepting an
    // invalid stream and later handing it to VLESS/Noise.
    let (a, b) = tokio::io::duplex(8192);
    let mut server =
        tokio_tungstenite::WebSocketStream::from_raw_socket(a, Role::Server, None).await;
    let sender = tokio::spawn(async move {
        let mut b = b;
        b.write_all(&[0x82, 1, 42]).await.unwrap();
    });
    assert!(server.next().await.unwrap().is_err());
    sender.await.unwrap();
}
