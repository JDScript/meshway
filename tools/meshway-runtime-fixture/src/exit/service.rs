use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    task::JoinSet,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Visit {
    protocol: &'static str,
    peer: std::net::SocketAddr,
    local: std::net::SocketAddr,
    bytes: usize,
}

pub(super) async fn run(cli: &Cli) -> Result<()> {
    let tcp = TcpListener::bind("0.0.0.0:8080").await?;
    let udp = UdpSocket::bind("0.0.0.0:8080").await?;
    // exit6/off: the same services on IPv6 (the container sets
    // `net.ipv6.bindv6only=1`, so these never see IPv4).
    let (tcp6, udp6) = if cli.traversal == crate::TraversalMode::Exit6 {
        (
            Some(TcpListener::bind("[::]:8080").await?),
            Some(UdpSocket::bind("[::]:8080").await?),
        )
    } else {
        (None, None)
    };
    let subscription = UdpSocket::bind("0.0.0.0:8081").await?;
    let mut subscriber = None;
    let mut sent = 0_u64;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tasks = JoinSet::new();
    let mut visits = Vec::new();
    let mut buffer = [0; 65536];
    let mut buffer6 = [0; 65536];
    let mut subscription_buffer = [0; 64];
    let result = async {
        loop {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? { break; }
            tokio::select! {
                accepted = tcp.accept(), if tasks.len() < 32 => {
                    let (mut stream, peer) = accepted?;
                    visits.push(Visit { protocol: "tcp", peer, local: stream.local_addr()?, bytes: 0 });
                    // Recorded before it is served: a client that got its
                    // answer always finds its visit in the log.
                    write(&cli.evidence.join("access.json"), &visits).await?;
                    tasks.spawn(async move {
                        let mut buffer = [0; 32768];
                        loop {
                            let count = match tokio::time::timeout(Duration::from_secs(150), stream.read(&mut buffer)).await {
                                Ok(Ok(count)) => count,
                                // Revocation/expiry can strand the external TCP
                                // peer until its idle deadline. Source assertions
                                // verify success; this service only owns cleanup.
                                Ok(Err(_)) | Err(_) => break,
                            };
                            if count == 0 { break; }
                            if !matches!(tokio::time::timeout(IO, stream.write_all(&buffer[..count])).await, Ok(Ok(()))) { break; }
                        }
                        Ok::<(), crate::Error>(())
                    });
                }
                packet = udp.recv_from(&mut buffer) => {
                    let (count, peer) = packet?;
                    visits.push(Visit { protocol: "udp", peer, local: udp.local_addr()?, bytes: count });
                    write(&cli.evidence.join("access.json"), &visits).await?;
                    let _ = udp.send_to(&buffer[..count], peer).await;
                }
                accepted = async { tcp6.as_ref().expect("guarded").accept().await }, if tcp6.is_some() && tasks.len() < 32 => {
                    let (mut stream, peer) = accepted?;
                    visits.push(Visit { protocol: "tcp", peer, local: stream.local_addr()?, bytes: 0 });
                    write(&cli.evidence.join("access.json"), &visits).await?;
                    tasks.spawn(async move {
                        let mut buffer = [0; 32768];
                        loop {
                            let count = match tokio::time::timeout(Duration::from_secs(150), stream.read(&mut buffer)).await {
                                Ok(Ok(count)) => count,
                                Ok(Err(_)) | Err(_) => break,
                            };
                            if count == 0 { break; }
                            if !matches!(tokio::time::timeout(IO, stream.write_all(&buffer[..count])).await, Ok(Ok(()))) { break; }
                        }
                        Ok::<(), crate::Error>(())
                    });
                }
                packet = async { udp6.as_ref().expect("guarded").recv_from(&mut buffer6).await }, if udp6.is_some() => {
                    let (count, peer) = packet?;
                    let socket = udp6.as_ref().expect("guarded");
                    visits.push(Visit { protocol: "udp", peer, local: socket.local_addr()?, bytes: count });
                    write(&cli.evidence.join("access.json"), &visits).await?;
                    let _ = socket.send_to(&buffer6[..count], peer).await;
                }
                packet = subscription.recv_from(&mut subscription_buffer) => {
                    let (count, peer) = packet?;
                    if &subscription_buffer[..count] != b"exit-subscribe" { return Err("invalid fixture subscription".into()); }
                    subscriber = Some((peer, std::time::Instant::now()));
                    tick.reset();
                    visits.push(Visit { protocol: "udp-subscription", peer, local: subscription.local_addr()?, bytes: count });
                }
                _ = tick.tick(), if subscriber.is_some() => {
                    if let Some((peer, started)) = subscriber {
                        if started.elapsed() < Duration::from_secs(150) {
                            if subscription.send_to(&sent.to_be_bytes(), peer).await.is_ok() {
                                sent += 1;
                                write(&cli.evidence.join("subscription-sent.json"), &sent).await?;
                            }
                        } else { subscriber = None; }
                    }
                }
                Some(done) = tasks.join_next(), if !tasks.is_empty() => { done??; }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            if visits.len() > 8192 { return Err("exit service log bound exceeded".into()); }
            write(&cli.evidence.join("access.json"), &visits).await?;
        }
        Ok::<(), crate::Error>(())
    }.await;
    tasks.abort_all();
    while let Some(done) = tasks.join_next().await {
        if let Err(error) = done
            && !error.is_cancelled()
        {
            return Err(error.into());
        }
    }
    result
}
