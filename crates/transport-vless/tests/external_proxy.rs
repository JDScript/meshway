//! Opt-in actual Xray/sing-box interoperability test. See README for a restricted
//! Xray config. These are wire-test bytes, not an authenticated Meshway session.

use std::{net::SocketAddr, time::Duration};

use meshway_transport_vless::{ClientConfig, SecretUuid, Target, client_stream};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

#[tokio::test]
#[ignore = "requires a separately running restricted Xray/sing-box server; see README"]
async fn external_vless_tcp_connect_binary_round_trip() {
    let uuid: SecretUuid = std::env::var("MESHWAY_VLESS_UUID")
        .expect("set MESHWAY_VLESS_UUID to the external server's temporary test credential")
        .parse()
        .expect("MESHWAY_VLESS_UUID must be a canonical UUID");
    let proxy: SocketAddr = std::env::var("MESHWAY_VLESS_PROXY")
        .unwrap_or_else(|_| "127.0.0.1:18443".into())
        .parse()
        .expect("MESHWAY_VLESS_PROXY must be a socket address");
    let bind: SocketAddr = std::env::var("MESHWAY_VLESS_BIND")
        .unwrap_or_else(|_| "127.0.0.1:19001".into())
        .parse()
        .expect("MESHWAY_VLESS_BIND must be a local probe socket address");
    let listener = TcpListener::bind(bind)
        .await
        .expect("the local probe bind address must be available and reachable from the proxy");
    let endpoint = listener.local_addr().unwrap();
    let target = match std::env::var("MESHWAY_VLESS_TARGET_HOST") {
        Ok(host) => match host.parse() {
            Ok(ip) => Target::ip(ip, endpoint.port()).unwrap(),
            Err(_) => Target::domain(&host, endpoint.port()).unwrap(),
        },
        Err(std::env::VarError::NotPresent) => Target::ip(endpoint.ip(), endpoint.port()).unwrap(),
        Err(_) => panic!("MESHWAY_VLESS_TARGET_HOST must be valid Unicode"),
    };
    let config = ClientConfig { uuid, target };
    let expect_deferred = std::env::var("MESHWAY_VLESS_EXPECT_DEFERRED").is_ok_and(|v| v == "1");
    let (connected_tx, connected_rx) = oneshot::channel();

    timeout(Duration::from_secs(10), async {
        let client = async {
            let tcp = TcpStream::connect(proxy).await.unwrap();
            let mut carrier = client_stream(tcp, &config).await.unwrap();
            connected_rx.await.unwrap();
            if expect_deferred {
                // This adapter retains partial response parsing state across
                // canceled reads. Verify the real proxy withholds its header
                // before the destination receives any client-first bytes.
                assert!(
                    timeout(Duration::from_millis(150), carrier.read_u8())
                        .await
                        .is_err()
                );
                assert!(
                    !carrier.response_validated(),
                    "the selected reference server unexpectedly sent an eager response"
                );
            }
            let outgoing: Vec<u8> = (0..4096).map(|n| (n % 256) as u8).collect();
            carrier.write_all(&outgoing).await.unwrap();
            carrier.flush().await.unwrap();
            let mut incoming = vec![0; outgoing.len()];
            carrier.read_exact(&mut incoming).await.unwrap();
            assert!(carrier.response_validated());
            assert_eq!(incoming, outgoing.iter().rev().copied().collect::<Vec<_>>());
            carrier.write_all(b"\0\xffdone").await.unwrap();
            carrier.shutdown().await.unwrap();
        };
        let private_destination = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            connected_tx.send(()).unwrap();
            let mut incoming = vec![0; 4096];
            stream.read_exact(&mut incoming).await.unwrap();
            assert_eq!(
                incoming,
                (0..4096).map(|n| (n % 256) as u8).collect::<Vec<_>>()
            );
            // Xray/sing-box may withhold the VLESS response until this write.
            incoming.reverse();
            stream.write_all(&incoming).await.unwrap();
            let mut ack = [0; 6];
            stream.read_exact(&mut ack).await.unwrap();
            assert_eq!(&ack, b"\0\xffdone");
        };
        tokio::join!(client, private_destination);
    })
    .await
    .expect("external VLESS request, deferred response, and round trip must finish");
}
