use super::*;
#[tokio::test]
async fn a_held_request_is_not_cut_and_only_quiet_connections_close() {
    // Scaled: 200 ms stands for IDLE.
    let idle = Duration::from_millis(200);
    let activity = Activity::default();
    // A held request: no byte for most of the idle time, then the answer.
    let quiet = activity.quiet(idle);
    tokio::pin!(quiet);
    tokio::select! {
        _ = &mut quiet => panic!("closed while a request was held"),
        _ = tokio::time::sleep(idle * 3 / 4) => activity.touch(),
    }
    // An active connection lives past any fixed age.
    let started = tokio::time::Instant::now();
    let alive = async {
        for _ in 0..10 {
            tokio::time::sleep(idle / 2).await;
            activity.touch();
        }
    };
    tokio::select! {
        _ = activity.quiet(idle) => panic!("an active connection closed"),
        _ = alive => {},
    }
    assert!(started.elapsed() >= idle * 5);
    let quiet_since = tokio::time::Instant::now();
    activity.quiet(idle).await;
    assert!(quiet_since.elapsed() >= idle * 9 / 10);
}

#[test]
fn families_alternate_from_the_resolvers_first_answer() {
    let addresses = |list: &[&str]| -> Vec<std::net::SocketAddr> {
        list.iter()
            .map(|address| address.parse().unwrap())
            .collect()
    };
    assert_eq!(
        super::interleave(addresses(&[
            "[2001:db8::1]:443",
            "[2001:db8::2]:443",
            "192.0.2.1:443",
            "192.0.2.2:443",
        ])),
        addresses(&[
            "[2001:db8::1]:443",
            "192.0.2.1:443",
            "[2001:db8::2]:443",
            "192.0.2.2:443",
        ])
    );
    assert_eq!(
        super::interleave(addresses(&[
            "192.0.2.1:443",
            "192.0.2.2:443",
            "[2001:db8::1]:443"
        ])),
        addresses(&["192.0.2.1:443", "[2001:db8::1]:443", "192.0.2.2:443"])
    );
    assert!(super::interleave(Vec::new()).is_empty());
}

fn directory() -> (tempfile::TempDir, Directory, PathBuf, std::fs::Metadata) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().canonicalize().unwrap().join("state");
    let directory = Directory::open(&path, "underlay.lock").unwrap();
    let socket = path.join("underlay-bootstrap.sock");
    drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let metadata = std::fs::symlink_metadata(&socket).unwrap();
    (root, directory, socket, metadata)
}

fn record(directory: &Directory, device: u64, inode: u64) {
    let bytes = serde_json::to_vec(&Record { device, inode }).unwrap();
    directory
        .replace("underlay-bootstrap.json", &bytes)
        .unwrap();
}

fn relay_socket(root: &tempfile::TempDir) -> Arc<Socket> {
    let path = root.path().canonicalize().unwrap().join("relay");
    let directory = Arc::new(Directory::open(&path, "underlay.lock").unwrap());
    let uid = std::fs::metadata(&path).unwrap().uid();
    Arc::new(Socket {
        path: path.join(crate::files::BOOTSTRAP_SOCKET),
        directory,
        uid,
        state: Default::default(),
    })
}

/// Echoes until the client closes.
fn echo(
    tasks: super::super::tasks::Tasks,
    limit: usize,
    uid: u32,
    state: State,
) -> Connections<
    impl Fn(UnixStream) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>,
> {
    Connections {
        tasks,
        permits: Arc::new(Semaphore::new(limit)),
        uid,
        refused: 0,
        state,
        relay:
            |stream: UnixStream| -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
                Box::pin(async move {
                    let (mut read, mut write) = stream.into_split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                })
            },
    }
}

async fn echoes(path: &std::path::Path) -> Option<UnixStream> {
    use tokio::io::AsyncWriteExt;
    let mut stream = UnixStream::connect(path).await.ok()?;
    stream.write_all(b"x").await.ok()?;
    let mut byte = [0; 1];
    match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await {
        Ok(Ok(1)) => Some(stream),
        _ => None,
    }
}

#[test]
fn listener_retries_follow_the_u1_cadence() {
    let delays = |error: &Error, attempts: usize| -> Vec<u64> {
        let mut recovery = relay_recovery();
        (0..attempts)
            .map(|_| failed(&mut recovery, "cannot listen", error).as_secs())
            .collect()
    };
    let busy = Error::Io(io::ErrorKind::AddrInUse.into());
    let mut waits = delays(&busy, 40);
    assert_eq!(waits[..8], [1, 2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(waits.pop(), Some(60));
    assert_eq!(delays(&Error::Conflict, 1), [1]);
    let denied = Error::Io(io::ErrorKind::PermissionDenied.into());
    assert_eq!(delays(&denied, 3), [60, 60, 60]);
}

/// X15/H4: a relay whose socket is still held (here a live listener on its
/// path, `EADDRINUSE` to a plain bind) — in a re-proved term or at process
/// start — keeps retrying on its own and listens once the path is free; the
/// term is not ended (the owner has no way to end it). Each attempt is
/// published (Status `controlRelay`), and listening clears it.
#[tokio::test]
async fn a_relay_that_cannot_listen_retries_until_its_socket_is_free() {
    let root = tempfile::tempdir().unwrap();
    let socket = relay_socket(&root);
    let held = UnixListener::bind(&socket.path).unwrap();
    std::fs::set_permissions(&socket.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let first = socket.bind().await;
    assert!(
        matches!(first, Err(Error::Conflict)),
        "a live socket is never reclaimed"
    );
    let tasks = super::super::tasks::Tasks::new(8);
    let (state, mut states) = tokio::sync::watch::channel(None);
    let owner = tokio::spawn(serve(
        socket.clone(),
        first,
        echo(tasks.clone(), 8, socket.uid, state),
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!owner.is_finished(), "the owner keeps retrying");
    match states.borrow_and_update().clone() {
        Some(crate::ControlRelayState::Retrying(pending)) => {
            assert_eq!(pending.attempts, 1);
            assert!(
                pending.reason.starts_with("cannot listen: "),
                "{}",
                pending.reason
            );
            assert!(pending.retry_at > std::time::SystemTime::now());
        }
        other => panic!("retrying is published, not {other:?}"),
    }
    drop(held);
    let started = std::time::Instant::now();
    let listening = loop {
        if let Some(stream) = echoes(&socket.path).await {
            break stream;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "listening after the next retry"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(*states.borrow(), None, "listening clears the retry");
    drop(listening);
    socket.close();
    assert!(
        !socket.path.exists(),
        "the owner removes the socket it bound"
    );
    owner.abort();
    let _ = tasks.shutdown().await;
}

/// X15: at the task capacity a new connection is refused and the current
/// one kept; once the capacity falls back, connections are admitted again.
#[tokio::test]
async fn a_relay_at_capacity_refuses_new_connections_and_keeps_current_ones() {
    let root = tempfile::tempdir().unwrap();
    let socket = relay_socket(&root);
    let first = socket.bind().await;
    assert!(first.is_ok());
    let tasks = super::super::tasks::Tasks::new(1);
    let (state, states) = tokio::sync::watch::channel(None);
    let connections = echo(tasks.clone(), 8, socket.uid, state);
    let owner = tokio::spawn(serve(socket.clone(), first, connections));
    let mut current = echoes(&socket.path)
        .await
        .expect("first connection relayed");
    assert_eq!(*states.borrow(), None);
    assert!(echoes(&socket.path).await.is_none(), "refused at capacity");
    assert_eq!(
        *states.borrow(),
        Some(crate::ControlRelayState::AtCapacity {
            limit: "tasks",
            refused: 1
        })
    );
    {
        use tokio::io::AsyncWriteExt;
        current.write_all(b"y").await.unwrap();
        let mut byte = [0; 1];
        current.read_exact(&mut byte).await.unwrap();
        assert_eq!(&byte, b"y", "the current connection is kept");
    }
    drop(current);
    let started = std::time::Instant::now();
    while echoes(&socket.path).await.is_none() {
        assert!(started.elapsed() < Duration::from_secs(5), "admitted again");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(*states.borrow(), None, "an admitted connection ends it");
    assert!(!owner.is_finished());
    owner.abort();
    let _ = tasks.shutdown().await;
}

#[tokio::test]
async fn stale_socket_on_a_renumbered_device_or_without_record_is_reclaimed() {
    let (_root, directory, socket, metadata) = directory();
    let uid = metadata.uid();
    record(&directory, metadata.dev().wrapping_add(1), metadata.ino());
    recover(&directory, &socket, uid).await.unwrap();
    assert!(!socket.exists());

    let (_root, directory, socket, metadata) = self::directory();
    recover(&directory, &socket, metadata.uid()).await.unwrap();
    assert!(!socket.exists());
}

#[tokio::test]
async fn a_different_inode_or_a_live_listener_is_never_reclaimed() {
    let (_root, directory, socket, metadata) = directory();
    let uid = metadata.uid();
    record(&directory, metadata.dev(), metadata.ino().wrapping_add(1));
    assert!(recover(&directory, &socket, uid).await.is_err());
    std::fs::remove_file(&socket).unwrap();
    let live = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let metadata = std::fs::symlink_metadata(&socket).unwrap();
    record(&directory, metadata.dev(), metadata.ino());
    assert!(recover(&directory, &socket, uid).await.is_err());
    assert!(socket.exists());
    drop(live);
}

/// L1: the Server's edge closing a relayed connection is not a relay
/// failure worth a WARN (field: 3–30 a day per Node, all `Broken pipe` or
/// `Connection reset by peer`); anything else still is.
#[test]
fn only_a_peer_closing_the_relay_is_quiet() {
    for kind in [
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::ConnectionAborted,
        io::ErrorKind::UnexpectedEof,
    ] {
        assert!(peer_closed(&kind.into()), "{kind:?}");
    }
    for kind in [
        io::ErrorKind::TimedOut,
        io::ErrorKind::ConnectionRefused,
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::Other,
    ] {
        assert!(!peer_closed(&kind.into()), "{kind:?}");
    }
}
