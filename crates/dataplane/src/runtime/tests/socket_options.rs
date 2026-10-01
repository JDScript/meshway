use super::*;
use socket2::SockRef;

#[tokio::test]
async fn native_tcp_nodelay_option_applies_to_dialed_and_accepted_sockets() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dial = tokio::net::TcpStream::connect(listener.local_addr().unwrap());
    let (dialed, accepted) = tokio::join!(dial, listener.accept());
    let dialed = dialed.unwrap();
    let (accepted, _) = accepted.unwrap();
    for value in [true, false] {
        for socket in [&dialed, &accepted] {
            stream::configure_socket(socket, value).unwrap();
            assert_eq!(socket.nodelay().unwrap(), value);
        }
    }
    let mut value = serde_json::to_value(configs().remove(0)).unwrap();
    value["limits"]
        .as_object_mut()
        .unwrap()
        .remove("native_tcp_nodelay");
    let defaulted: LabConfig = serde_json::from_value(value.clone()).unwrap();
    assert!(defaulted.limits.native_tcp_nodelay);
    value["limits"]["native_tcp_nodelay"] = serde_json::json!(false);
    let explicit: LabConfig = serde_json::from_value(value).unwrap();
    assert!(!explicit.limits.native_tcp_nodelay);
}

#[test]
fn udp_receive_buffer_config_preserves_default_and_bounds_requests() {
    let mut value = serde_json::to_value(configs().remove(0)).unwrap();
    value["limits"]
        .as_object_mut()
        .unwrap()
        .remove("udp_receive_buffer_bytes");
    let defaulted: LabConfig = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(defaulted.limits.udp_receive_buffer_bytes, 0);
    defaulted.validate().unwrap();

    for request in [0, 1, 1024 * 1024, 4 * 1024 * 1024] {
        value["limits"]["udp_receive_buffer_bytes"] = serde_json::json!(request);
        let config: LabConfig = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(config.limits.udp_receive_buffer_bytes, request);
        config.validate().unwrap();
    }
    value["limits"]["udp_receive_buffer_bytes"] = serde_json::json!(4 * 1024 * 1024 + 1);
    let oversized: LabConfig = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(
        oversized.validate(),
        Err(Error::InvalidConfig("invalid runtime limits"))
    );
    for invalid in [serde_json::json!(-1), serde_json::json!(1.5)] {
        value["limits"]["udp_receive_buffer_bytes"] = invalid;
        assert!(serde_json::from_value::<LabConfig>(value.clone()).is_err());
    }
}

#[tokio::test]
async fn udp_listener_and_outgoing_bind_apply_only_requested_receive_buffer() {
    let address: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let reference = UdpSocket::bind(address).await.unwrap();
    let reference_options = SockRef::from(&reference);
    let default_receive = reference_options.recv_buffer_size().unwrap();
    let default_send = reference_options.send_buffer_size().unwrap();
    let candidate = OutgoingCandidate {
        peer: "lax".into(),
        transport: TransportKind::NativeUdp,
        endpoint: reference.local_addr().unwrap().to_string(),
        priority: 0,
        id: None,
    };

    for socket in [
        udp::bind(address, 0).unwrap(),
        udp::bind_outgoing(&candidate, 0).unwrap(),
    ] {
        let options = SockRef::from(&socket);
        assert_eq!(options.recv_buffer_size().unwrap(), default_receive);
        assert_eq!(options.send_buffer_size().unwrap(), default_send);
    }

    // Compare with this platform's direct SO_RCVBUF result, not with the request
    // or a Linux-specific doubling rule. Even a successful set may be clamped.
    const REQUEST: usize = 16 * 1024;
    reference_options.set_recv_buffer_size(REQUEST).unwrap();
    let expected = reference_options.recv_buffer_size().unwrap();
    for socket in [
        udp::bind(address, REQUEST).unwrap(),
        udp::bind_outgoing(&candidate, REQUEST).unwrap(),
    ] {
        let options = SockRef::from(&socket);
        assert_eq!(options.recv_buffer_size().unwrap(), expected);
        assert_eq!(options.send_buffer_size().unwrap(), default_send);
        assert_ne!(socket.local_addr().unwrap().port(), 0);
        // The configured socket is still nonblocking and usable by Tokio.
        socket
            .send_to(b"configured", reference.local_addr().unwrap())
            .await
            .unwrap();
        let mut bytes = [0; 32];
        let (size, _) =
            tokio::time::timeout(Duration::from_secs(1), reference.recv_from(&mut bytes))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(&bytes[..size], b"configured");
    }
}
