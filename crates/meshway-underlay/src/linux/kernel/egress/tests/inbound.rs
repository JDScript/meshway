//! X19: a connection the host accepted on the underlay keeps its replies on
//! the underlay under a full capture, while the gate is paused, and after
//! a table of an earlier version gains the exception; connections the host
//! opens stay captured or rejected; `down` removes everything.
use super::*;
use tokio::net::{TcpListener, TcpStream};

/// The external fixture peer opens a TCP connection to `port` on this host
/// when asked by one datagram, then echoes what it reads.
async fn dialed(
    kernel: &Kernel,
    listener: &TcpListener,
    external: SocketAddr,
    protected: bool,
) -> TcpStream {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
    if protected {
        kernel
            .shared
            .prepare(&socket2::SockRef::from(&socket))
            .unwrap();
    }
    let port = listener.local_addr().unwrap().port();
    socket
        .send_to(format!("dial:{port}").as_bytes(), external)
        .unwrap();
    let (stream, peer) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .expect("the external peer connects in")
        .unwrap();
    assert_eq!(peer.ip(), external.ip());
    stream
}

async fn echoes(stream: &mut TcpStream, body: &[u8], within: Duration) -> bool {
    tokio::time::timeout(within, async {
        stream.write_all(body).await.unwrap();
        let mut reply = vec![0; body.len()];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, body);
    })
    .await
    .is_ok()
}

/// Small exchanges on `stream` for `seconds`, each answered within 2 s.
async fn sustained(stream: &mut TcpStream, label: &str, seconds: u64) {
    let end = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut count = 0u32;
    while tokio::time::Instant::now() < end {
        let body = format!("{label}-{count}");
        assert!(
            echoes(stream, body.as_bytes(), Duration::from_secs(2)).await,
            "{label}: exchange {count} was not answered"
        );
        count += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    println!("X19_SUSTAINED {label} exchanges={count} seconds={seconds}");
}

fn nft_table(table: &str) -> Option<serde_json::Value> {
    let output = std::process::Command::new("/usr/sbin/nft")
        .args(["-j", "list", "table", "inet", table])
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| serde_json::from_slice(&output.stdout).unwrap())
}

fn has_replies_chain(table: &str) -> bool {
    nft_table(table).is_some_and(|listed| {
        listed["nftables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["chain"]["name"] == "replies")
    })
}

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, TUN, nft, NET_ADMIN and an external dialing peer"]
async fn accepted_connections_keep_underlay_replies_through_capture_pause_and_upgrade() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let external: SocketAddr = std::env::var("MESHWAY_X4_EXTERNAL")
        .unwrap()
        .parse()
        .unwrap();
    let Setup {
        _temp,
        tasks,
        mut kernel,
        mut backend,
        device,
        mesh,
        ..
    } = setup_with(false).await;
    let table = backend.ledger.program.table();
    assert!(!has_replies_chain(&table), "a table of an earlier version");
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let mut early = dialed(&kernel, &listener, external, false).await;
    assert!(echoes(&mut early, b"before-capture", Duration::from_secs(2)).await);

    // The defect, on the earlier table: a full capture takes the replies of
    // the accepted connection into the TUN.
    let full = full();
    backend.stage(full.clone(), mesh.clone()).await.unwrap();
    backend
        .commit_staged(full.clone(), mesh.clone(), Some(device.clone()), || Ok(()))
        .await
        .unwrap();
    assert!(
        !echoes(&mut early, b"lost-reply", Duration::from_millis(1500)).await,
        "the earlier table captures the replies of an accepted connection"
    );
    println!("X19_EARLIER_TABLE=accepted-connection-replies-captured");

    // The owner opening the table again adds the exception in one
    // transaction; the connection's own retransmission then reaches the
    // peer, without a new connection.
    backend.initialize().await.unwrap();
    assert!(backend.ledger.program.inbound);
    assert!(has_replies_chain(&table));
    backend.verify().await.unwrap();
    let mut late = vec![0; b"lost-reply".len()];
    tokio::time::timeout(Duration::from_secs(15), early.read_exact(&mut late))
        .await
        .expect("the retransmitted reply reaches the peer once the exception is installed")
        .unwrap();
    assert_eq!(late, b"lost-reply");
    println!("X19_UPGRADE=chain-added,existing-connection-recovered");

    // Under the full capture: the accepted connection, and one accepted
    // now, are answered on the underlay; a connection the host opens is
    // captured.
    let mut fresh = dialed(&kernel, &listener, external, true).await;
    sustained(&mut early, "capture-early", 10).await;
    sustained(&mut fresh, "capture-fresh", 5).await;
    let opened = SocketAddr::new(external.ip(), 18081);
    captured(kernel.tun.as_ref().unwrap(), opened).await;
    println!("X19_CAPTURE=accepted-answered,opened-captured");

    // The negative boundary (paused gate, as on withdrawal or lease
    // expiry): accepted connections stay answered; one the host opens is
    // rejected at once.
    backend.pause().await.unwrap();
    backend.verify().await.unwrap();
    sustained(&mut early, "paused-early", 10).await;
    sustained(&mut fresh, "paused-fresh", 3).await;
    let refused = tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(opened)).await;
    assert!(
        refused.is_ok_and(|result| result.is_err()),
        "a connection the host opens is rejected by the negative boundary"
    );
    println!("X19_PAUSED=accepted-answered,opened-rejected");

    // A restart of the owner reads the table back unchanged.
    backend.initialize().await.unwrap();
    backend.verify().await.unwrap();

    backend.retire().await.unwrap();
    kernel.quiesce().await.unwrap();
    kernel.down().await.unwrap();
    assert!(
        nft_table(&table).is_none(),
        "down removes the capture table"
    );
    let rules = std::process::Command::new("/usr/sbin/ip")
        .args(["rule", "show"])
        .output()
        .unwrap();
    println!(
        "X19_RULES_AFTER_DOWN={}",
        String::from_utf8_lossy(&rules.stdout)
    );
    assert!(echoes(&mut early, b"after-down", Duration::from_secs(2)).await);
    tasks.shutdown().await.unwrap();
    println!("X19_DOWN=table-removed,accepted-connection-kept");
}
