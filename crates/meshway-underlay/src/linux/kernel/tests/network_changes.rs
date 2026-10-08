use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "isolated Linux namespace, /dev/net/tun and NET_ADMIN required"]
async fn owned_suspend_keeps_control_connection_and_reports_no_network_change() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("node"), "fixture.lock").unwrap());
    let mut host = crate::Host::open(directory, Options::default())
        .await
        .unwrap();
    let mut changes = host.network_changes();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    // All four requests must traverse the same real protected pooled stream.
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        for turn in 0..4 {
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
                assert!(request.len() < 4096);
            }
            let response: &[u8] = if turn == 2 {
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\n\r\nok"
            } else {
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok"
            };
            stream.write_all(response).await.unwrap();
        }
    });
    host.attach_control(&origin).await.unwrap();
    let client = host
        .client_builder()
        .unwrap()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    async fn request(client: &reqwest::Client, origin: &str, status: u16) {
        let response = client.get(origin).send().await.unwrap();
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.text().await.unwrap(), "ok");
    }
    request(&client, &origin, 200).await;
    let pools = ["100.96.0.0/16".parse().unwrap()];
    host.kernel.retain_negative(&pools).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "negative rules caused external change notification"
    );
    request(&client, &origin, 200).await;
    let addresses = meshway_configuration::NodeAddresses {
        ipv4: Some("100.96.0.1".parse().unwrap()),
        ipv6: None,
    };
    {
        let kernel = &mut host.kernel;
        let mut expected = kernel.expected.lock().unwrap();
        expected.changing = true;
        let tun = Tun::create(
            meshway_tun::Options::new(1280)
                .unwrap()
                .with_ipv4(addresses.ipv4.unwrap(), 32)
                .unwrap(),
        )
        .unwrap();
        let index = tun.if_index().unwrap();
        expected.tun = Some((
            index,
            tun.name().into(),
            tun.mtu(),
            vec![addresses.ipv4.unwrap().into()],
        ));
        kernel.journal.tun = Some(TunRecord {
            name: tun.name().into(),
            ifindex: index,
            mtu: tun.mtu(),
            addresses: addresses.clone(),
        });
        kernel.tun = Some(tun);
    }
    let index = host.kernel.tun.as_ref().unwrap().if_index().unwrap();
    host.kernel.address_ready(index, &addresses).await.unwrap();
    host.kernel.quiesce().await.unwrap();
    host.kernel.expected.lock().unwrap().changing = false;
    for route in plan::positive_routes(&pools, &addresses, host.kernel.journal.table, index) {
        host.kernel.add_route(route).await.unwrap();
    }
    host.kernel.quiesce().await.unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "owned TUN creation caused external change notification"
    );
    host.suspend().await.unwrap();
    host.kernel.quiesce().await.unwrap();
    assert!(
        !changes.has_changed().unwrap(),
        "owned TUN removal caused external change notification"
    );
    // Real OS denial is not fed back as an external interface observation.
    let error = tokio::net::TcpStream::connect("100.96.0.2:42425")
        .await
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::HostUnreachable
    ));
    assert!(!changes.has_changed().unwrap());
    assert!(host.kernel.expected.lock().unwrap().retiring_tun.is_none());
    request(&client, &origin, 503).await;
    host.down().await.unwrap();
    host.kernel.quiesce().await.unwrap();
    assert!(!changes.has_changed().unwrap());
    request(&client, &origin, 200).await;
    server.await.unwrap();
    host.shutdown(false).await.unwrap();
    changes.borrow_and_update();
}
