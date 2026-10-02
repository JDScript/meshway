use super::*;
struct Unavailable;
impl Provider for Unavailable {
    fn live_status(&self, _: bool, _: u32, _: u32) -> Result<LiveStatus, ProbeError> {
        Err(ProbeError::Unavailable)
    }
    fn whois(&self, _: &str) -> Result<WhoisObservation, ProbeError> {
        Err(ProbeError::Unavailable)
    }
    fn probe_neighbor_observed(
        &self,
        _: ProbeRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ProbeObservation, ProbeError>> + Send + '_>> {
        Box::pin(async { Err(ProbeError::Unavailable) })
    }
}

#[tokio::test]
async fn versioned_requests_are_closed_and_response_bound_is_independent() {
    for body in [
        r#"{"schema_version":1,"operation":{"command":"dns_status","extra":true}}"#,
        r#"{"schema_version":1,"operation":{"command":"status","peers":false,"offset":0,"limit":64,"extra":true}}"#,
        r#"{"schema_version":1,"operation":{"command":"whois","query":"SHA"},"extra":true}"#,
        r#"{"schema_version":1,"operation":{"command":"dns_query","query":"SHA"}}"#,
        r#"{"schema_version":1,"operation":{"command":"ping","request":{"peer":"SHA","timeout_ms":3000,"extra":true}}}"#,
        r#"{"schema_version":1,"operation":{"command":"ping","request":{"peer":"SHA","timeout_ms":3000},"extra":true}}"#,
        r#"{"schema_version":1,"schema_version":2,"operation":{"command":"whois","query":"SHA"}}"#,
    ] {
        assert!(serde_json::from_str::<Incoming>(body).is_err());
    }
    let dir = directory();
    let state = dir.path().join("a");
    let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
    let prober = Arc::new(Unavailable);
    let server = Server::bind(&state, &lock).await.unwrap();
    let task = tokio::spawn(async move { server.serve(prober).await });
    for version in [0, 2, u32::MAX] {
        let response: LiveResponse = exchange(
            &state,
            &Request {
                schema_version: version,
                operation: Operation::Whois {
                    query: "SHA".into(),
                },
            },
            MAX_RESPONSE,
        )
        .await
        .unwrap();
        assert!(matches!(
            response,
            LiveResponse::Error(ProbeError::UnsupportedVersion)
        ));
    }
    assert_eq!(
        live_status(&state, false, 0, 64).await.unwrap_err(),
        ProbeError::Unavailable
    );
    assert_eq!(
        whois(&state, "SHA").await.unwrap_err(),
        ProbeError::Unavailable
    );
    let dns = dns::status(&state).await.unwrap();
    assert_eq!(dns.state, dns::State::Unavailable);
    assert!(!dns.listener_bound);
    assert!(dns.listener.is_none());
    task.abort();
    let _ = task.await;

    let large = "x".repeat(5000);
    assert!(frame(&large).is_err());
    let (mut a, mut b) = UnixStream::pair().unwrap();
    a.write_all(&bounded_frame(&large, MAX_RESPONSE).unwrap())
        .await
        .unwrap();
    assert_eq!(
        read_bounded_frame::<String>(&mut b, MAX_RESPONSE)
            .await
            .unwrap(),
        large
    );
    let (mut a, mut b) = UnixStream::pair().unwrap();
    a.write_u32(MAX_RESPONSE as u32 + 1).await.unwrap();
    assert!(matches!(
        read_bounded_frame::<LiveResponse>(&mut b, MAX_RESPONSE).await,
        Err(ProbeError::InvalidRequest)
    ));
}

fn directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("mw")
        .permissions(Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[tokio::test]
async fn framing_fragmentation_unknown_fields_and_oversize() {
    let (mut a, mut b) = UnixStream::pair().unwrap();
    let request = ProbeRequest {
        peer: "sha".into(),
        timeout_ms: 3000,
    };
    let bytes = frame(&request).unwrap();
    let writer = tokio::spawn(async move {
        for chunk in bytes.chunks(3) {
            a.write_all(chunk).await.unwrap();
        }
    });
    let decoded: ProbeRequest = read_frame(&mut b).await.unwrap();
    assert_eq!(decoded.peer, "sha");
    writer.await.unwrap();

    for body in [
        br#"{"peer":"sha","timeout_ms":1000,"command":"exec"}"#.as_slice(),
        br#"{"peer":"sha","timeout_ms":1000} {}"#.as_slice(),
        br#"{"peer":"sha","timeout_ms":1000,"peer":"lax"}"#.as_slice(),
    ] {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_u32(body.len() as u32).await.unwrap();
        a.write_all(body).await.unwrap();
        assert!(matches!(
            read_frame::<ProbeRequest>(&mut b).await,
            Err(ProbeError::InvalidRequest)
        ));
    }
    for length in [0, 4097, u32::MAX] {
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_u32(length).await.unwrap();
        // A huge header alone must fail without waiting for or allocating a body.
        assert!(matches!(
            time::timeout(
                Duration::from_millis(100),
                read_frame::<ProbeRequest>(&mut b)
            )
            .await
            .unwrap(),
            Err(ProbeError::InvalidRequest)
        ));
    }
    let (mut a, mut b) = UnixStream::pair().unwrap();
    a.write_u32(30).await.unwrap();
    a.write_all(b"short").await.unwrap();
    drop(a);
    assert!(read_frame::<ProbeRequest>(&mut b).await.is_err());
    for timeout_ms in [0, 999, 10_001, u64::MAX] {
        assert!(
            ProbeRequest {
                peer: "sha".into(),
                timeout_ms
            }
            .validate()
            .is_err()
        );
    }
}

#[tokio::test]
async fn socket_permissions_stale_recovery_refusals_and_owned_inode_cleanup() {
    let dir = directory();
    let state = dir.path().join("a");
    let path = socket_path(&state);
    let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
    assert_eq!(
        probe(&state, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Unavailable
    );
    fs::write(&path, b"do not replace").unwrap();
    assert!(Server::bind(&state, &lock).await.is_err());
    assert_eq!(fs::read(&path).unwrap(), b"do not replace");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&state, &path).unwrap();
    assert!(Server::bind(&state, &lock).await.is_err());
    assert!(
        fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&path).unwrap();
    let stale = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(stale);
    let server = Server::bind(&state, &lock).await.unwrap();
    assert_eq!(fs::symlink_metadata(&path).unwrap().mode() & 0o777, 0o600);
    let active_inode = fs::symlink_metadata(&path).unwrap().ino();
    assert!(
        Server::bind(&state, &lock).await.is_err(),
        "active socket must not be replaced"
    );
    assert_eq!(fs::symlink_metadata(&path).unwrap().ino(), active_inode);
    // A replacement inode must survive the old owner's Drop.
    fs::remove_file(&path).unwrap();
    let replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(server);
    assert!(path.exists());
    drop(replacement);
    let server = Server::bind(&state, &lock).await.unwrap();
    drop(server);
    assert!(!path.exists());

    let long_state = dir.path().join("x".repeat(110));
    assert!(
        Server::bind(&long_state, &lock)
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("too long")
    );
    fs::set_permissions(dir.path(), Permissions::from_mode(0o755)).unwrap();
    assert!(Server::bind(&state, &lock).await.is_err());
    fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn saturated_accept_backlog_is_bounded_and_never_unlinked() {
    use rustix::net::{AddressFamily, SocketAddrUnix, SocketType};

    let dir = directory();
    let state = dir.path().join("a");
    let path = socket_path(&state);
    let lock_path = dir.path().join("a.lock");
    let lock = meshway_protocol::local::lock(&lock_path).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    // Rust's default listen backlog is platform/version dependent. Set it
    // explicitly before connecting; Linux may admit backlog + 1 connections.
    rustix::net::listen(&listener, 1).unwrap();
    fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    let original = fs::symlink_metadata(&path).unwrap();

    // Observe the actual nonblocking connect syscall result, not an async
    // wrapper's provisional success. Confirm both peer address and byte delivery
    // before counting a client, retain its fd, and never drain the accept queue.
    let address = SocketAddrUnix::new(&path).unwrap();
    let mut clients = Vec::new();
    let mut saturated = false;
    for _ in 0..8 {
        let fd = rustix::net::socket(AddressFamily::UNIX, SocketType::STREAM, None).unwrap();
        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC).unwrap();
        let mut client = std::os::unix::net::UnixStream::from(fd);
        client.set_nonblocking(true).unwrap();
        match rustix::net::connect(&client, &address).map_err(io::Error::from) {
            Ok(()) => {
                assert_eq!(
                    client.peer_addr().unwrap().as_pathname(),
                    Some(path.as_path())
                );
                assert_eq!(std::io::Write::write(&mut client, b"q").unwrap(), 1);
                clients.push(client);
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || (cfg!(target_os = "macos")
                        && error.kind() == io::ErrorKind::ConnectionRefused) =>
            {
                saturated = true;
                break;
            }
            Err(error) => panic!("could not saturate test socket backlog: {error}"),
        }
    }
    assert!(saturated, "fixture must fill the actual accept backlog");
    assert!(
        !clients.is_empty(),
        "test listener must have accepted connections into its backlog"
    );

    // An async timeout alone cannot detect a blocking syscall on this test's
    // single-thread executor. An independent watchdog frees a slot after two
    // seconds so the old blocking implementation fails the wall-clock bound
    // rather than hanging the test process forever.
    let rescue = listener.try_clone().unwrap();
    let (finished, completion) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if matches!(
            completion.recv_timeout(Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ) {
            let _ = rescue.accept();
        }
    });
    let started = std::time::Instant::now();
    let result = time::timeout(Duration::from_secs(1), Server::bind(&state, &lock)).await;
    let elapsed = started.elapsed();
    let _ = finished.send(());
    watchdog.join().unwrap();
    assert!(
        elapsed < Duration::from_secs(1),
        "startup stale check blocked for {elapsed:?}"
    );
    let error = result
        .expect("startup stale check exceeded its deadline")
        .err()
        .expect("an active or unverifiable socket must never be replaced");
    assert!(error.to_string().contains("cannot be verified stale"));
    let retained = fs::symlink_metadata(&path).unwrap();
    assert_eq!(
        (retained.dev(), retained.ino()),
        (original.dev(), original.ino())
    );
    assert_eq!(retained.uid(), original.uid());
    assert_eq!(retained.mode(), original.mode());
    assert!(meshway_protocol::local::lock(&lock_path).is_err());
    // Every counted connection really reached the original accept queue, and
    // that listener remains usable after the rejected startup.
    for _ in 0..clients.len() {
        let (mut accepted, _) = listener.accept().unwrap();
        accepted.set_nonblocking(true).unwrap();
        let mut byte = [0];
        std::io::Read::read_exact(&mut accepted, &mut byte).unwrap();
        assert_eq!(&byte, b"q");
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn bounded_connections_deadline_and_cancellation() {
    let dir = directory();
    let state = dir.path().join("a");
    let path = socket_path(&state);
    let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
    let prober = Arc::new(Unavailable);
    let server = Server::bind(&state, &lock).await.unwrap();
    let task = tokio::spawn(async move { server.serve(prober).await });
    let mut stalled = Vec::new();
    for _ in 0..MAX_CONNECTIONS {
        let mut socket = UnixStream::connect(&path).await.unwrap();
        socket.write_u32(100).await.unwrap(); // incomplete payload holds the slot
        stalled.push(socket);
    }
    time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        probe(&state, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Busy
    );
    // Client cancellation releases slots without waiting twelve seconds.
    stalled.clear();
    time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        probe(&state, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Unavailable
    );
    task.abort();
    let _ = task.await;
    assert!(!path.exists());
}

#[tokio::test]
async fn request_rate_is_bounded_even_for_fast_failures() {
    let dir = directory();
    let state = dir.path().join("a");
    let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
    let prober = Arc::new(Unavailable);
    let server = Server::bind(&state, &lock).await.unwrap();
    let task = tokio::spawn(async move { server.serve(prober).await });
    for _ in 0..8 {
        assert_eq!(
            probe(&state, "sha", Duration::from_secs(1))
                .await
                .unwrap_err(),
            ProbeError::Unavailable
        );
    }
    assert_eq!(
        probe(&state, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Busy
    );
    time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        probe(&state, "sha", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Unavailable
    );
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn incomplete_request_deadline_closes_connection_without_stopping_service() {
    let dir = directory();
    let state = dir.path().join("a");
    let lock = meshway_protocol::local::lock(&dir.path().join("a.lock")).unwrap();
    let prober = Arc::new(Unavailable);
    let server = Server::bind(&state, &lock).await.unwrap();
    let task = tokio::spawn(async move { server.serve(prober).await });
    let mut client = UnixStream::connect(socket_path(&state)).await.unwrap();
    client.write_u32(4096).await.unwrap();
    let mut byte = [0];
    let read = time::timeout(
        CONNECTION_DEADLINE + Duration::from_secs(1),
        client.read(&mut byte),
    )
    .await
    .unwrap();
    assert!(
        matches!(read, Ok(0)),
        "incomplete frame must be closed by the server deadline"
    );
    assert_eq!(
        probe(&state, "SHA", Duration::from_secs(1))
            .await
            .unwrap_err(),
        ProbeError::Unavailable
    );
    task.abort();
    let _ = task.await;
}
