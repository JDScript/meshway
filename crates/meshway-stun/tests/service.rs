use std::{net::SocketAddr, time::Duration};

use meshway_stun::{
    Error, Fingerprint, MAX_DATAGRAM_SIZE, Service, ServiceConfig, TransactionId, encode_request,
    parse_response,
};
use tokio::{net::UdpSocket, task::JoinHandle, time::timeout};
use tokio_util::sync::CancellationToken;

const TX: TransactionId = TransactionId::from_bytes([7; 12]);
const DEADLINE: Duration = Duration::from_secs(2);

async fn start(
    bind: &str,
    config: ServiceConfig,
) -> (
    SocketAddr,
    CancellationToken,
    JoinHandle<std::io::Result<meshway_stun::ServiceStats>>,
) {
    let socket = UdpSocket::bind(bind).await.unwrap();
    let address = socket.local_addr().unwrap();
    let cancellation = CancellationToken::new();
    let service = Service::new(socket, config).unwrap();
    let task = tokio::spawn(service.run(cancellation.clone()));
    (address, cancellation, task)
}

async fn receive(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut bytes = vec![0; MAX_DATAGRAM_SIZE + 1];
    let (length, source) = timeout(DEADLINE, socket.recv_from(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    bytes.truncate(length);
    (bytes, source)
}

async fn stop(
    cancellation: CancellationToken,
    task: JoinHandle<std::io::Result<meshway_stun::ServiceStats>>,
) -> meshway_stun::ServiceStats {
    cancellation.cancel();
    timeout(DEADLINE, task).await.unwrap().unwrap().unwrap()
}

#[tokio::test]
async fn real_loopback_both_families_and_both_fingerprint_modes() {
    for bind in ["127.0.0.1:0", "[::1]:0"] {
        let (server, cancel, task) = start(bind, ServiceConfig::default()).await;
        let client = UdpSocket::bind(bind).await.unwrap();
        let actual_client = client.local_addr().unwrap();
        for fingerprint in [Fingerprint::Omit, Fingerprint::Include] {
            let request = encode_request(TX, fingerprint);
            client.send_to(&request, server).await.unwrap();
            let (response, actual_source) = receive(&client).await;
            let mapped = parse_response(TX, server, actual_source, &response).unwrap();
            assert_eq!(mapped.socket_addr(), actual_client);
            assert!(response.len() * 5 <= request.len() * 11);
            assert_eq!(
                parse_response(
                    TransactionId::from_bytes([8; 12]),
                    server,
                    actual_source,
                    &response
                ),
                Err(Error::Transaction)
            );
            // A second socket's identical bytes do not correlate to the endpoint.
            let spoof = UdpSocket::bind(bind).await.unwrap();
            spoof.send_to(&response, actual_client).await.unwrap();
            let (bytes, actual_source) = receive(&client).await;
            assert_eq!(
                parse_response(TX, server, actual_source, &bytes),
                Err(Error::Source)
            );
        }
        let stats = stop(cancel, task).await;
        assert_eq!(stats.received_datagrams, 2);
        assert_eq!(stats.sent_datagrams, 2);
        assert_eq!(stats.peak_source_ips, 1);
        // Completion owns shutdown: the listening socket really was released.
        let rebound = UdpSocket::bind(server).await.unwrap();
        drop(rebound);
    }
}

#[tokio::test]
async fn garbage_truncation_and_auth_are_silent_then_valid_request_survives() {
    let (server, cancel, task) = start(
        "127.0.0.1:0",
        ServiceConfig {
            per_ip_datagrams_per_second: 32,
            ..ServiceConfig::default()
        },
    )
    .await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = encode_request(TX, Fingerprint::Omit);
    let mut indication = request.clone();
    indication[1] = 0x11;
    let mut response = request.clone();
    response[0] = 1;
    let mut auth = request.clone();
    auth[3] = 4;
    auth.extend_from_slice(&[0, 6, 0, 0]);
    let mut malformed = request.clone();
    malformed[3] = 4;
    malformed.extend_from_slice(&[0x80, 0x22, 0xff, 0xff]);
    for bytes in [
        vec![],
        vec![0xff; 19],
        vec![0; MAX_DATAGRAM_SIZE],
        vec![0; MAX_DATAGRAM_SIZE + 1],
        vec![0; MAX_DATAGRAM_SIZE * 4],
        indication,
        response,
        auth,
        malformed,
    ] {
        client.send_to(&bytes, server).await.unwrap();
    }
    client.send_to(&request, server).await.unwrap();
    let (bytes, actual_source) = receive(&client).await;
    assert_eq!(
        parse_response(TX, server, actual_source, &bytes)
            .unwrap()
            .socket_addr(),
        client.local_addr().unwrap()
    );
    let mut extra = [0; 128];
    assert!(
        timeout(Duration::from_millis(30), client.recv_from(&mut extra))
            .await
            .is_err()
    );
    let stats = stop(cancel, task).await;
    assert_eq!(stats.received_datagrams, 10);
    assert_eq!(stats.empty_datagrams, 1);
    assert_eq!(stats.oversized_datagrams, 2);
    assert_eq!(stats.invalid_datagrams, 6);
    assert_eq!(stats.sent_datagrams, 1);
}

#[tokio::test]
async fn unknown_mandatory_is_a_small_420_without_attribute_echo() {
    let (server, cancel, task) = start("127.0.0.1:0", ServiceConfig::default()).await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut request = encode_request(TX, Fingerprint::Omit);
    request[3] = 12;
    request.extend_from_slice(&[0x12, 0x34, 0, 8]);
    request.extend_from_slice(b"password");
    client.send_to(&request, server).await.unwrap();
    let (bytes, source) = receive(&client).await;
    assert_eq!(
        parse_response(TX, server, source, &bytes),
        Err(Error::ErrorResponse { code: 420 })
    );
    assert!(!bytes.windows(8).any(|w| w == b"password"));
    assert!(bytes.len() * 5 <= request.len() * 11);
    assert_eq!(stop(cancel, task).await.sent_datagrams, 1);
}

#[tokio::test]
async fn flood_is_bounded_and_cancel_stops_queued_work() {
    let (server, cancel, task) = start(
        "127.0.0.1:0",
        ServiceConfig {
            max_source_ips: 1,
            global_datagrams_per_second: 8,
            per_ip_datagrams_per_second: 2,
            global_output_bytes_per_second: 32,
        },
    )
    .await;
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let request = encode_request(TX, Fingerprint::Omit);
    for _ in 0..32 {
        client.send_to(&request, server).await.unwrap();
    }
    let (bytes, source) = receive(&client).await;
    assert!(parse_response(TX, server, source, &bytes).is_ok());
    let mut extra = [0; 128];
    assert!(
        timeout(Duration::from_millis(30), client.recv_from(&mut extra))
            .await
            .is_err()
    );
    let stats = stop(cancel, task).await;
    assert_eq!(stats.sent_datagrams, 1);
    assert_eq!(stats.sent_bytes, 32);
    assert_eq!(stats.output_limited_datagrams, 1);
    assert!(stats.rate_limited_datagrams > 0);
    assert_eq!(stats.peak_source_ips, 1);
}

#[tokio::test]
async fn already_cancelled_service_never_replies_and_drop_releases_socket() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let server = socket.local_addr().unwrap();
    let service = Service::new(socket, ServiceConfig::default()).unwrap();
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client
        .send_to(&encode_request(TX, Fingerprint::Omit), server)
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        service.run(cancel).await.unwrap(),
        meshway_stun::ServiceStats::default()
    );
    let rebound = UdpSocket::bind(server).await.unwrap();
    let future = Service::new(rebound, ServiceConfig::default())
        .unwrap()
        .run(CancellationToken::new());
    drop(future);
    drop(UdpSocket::bind(server).await.unwrap());
}
