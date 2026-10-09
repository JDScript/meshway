use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};
const WARMUP: &[u8] = b"meshway-exit-encrypted-through-r";

pub(super) async fn run(cli: &Cli, topology: &Topology) -> Result<()> {
    let mut tuples = Vec::new();
    let mut flows = Vec::new();
    for link in ["external", "lan"] {
        let address = topology.addresses[link][link];
        let udp = initial_udp(address, 8080).await?;
        let payload = WARMUP;
        let mut received = [0; 2048];
        // Demand creation may hold/drop the initial packet. Retry only initial
        // admission; steady traffic and all later stages have exact deadlines.
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if udp.send(payload).await.is_err() {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                if let Ok(Ok(count)) =
                    tokio::time::timeout(Duration::from_secs(1), udp.recv(&mut received)).await
                {
                    if &received[..count] != payload {
                        return Err("exit UDP payload mismatch".into());
                    }
                    break;
                }
            }
            Ok::<(), crate::Error>(())
        })
        .await??;
        let tcp = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                if let Ok(Ok(stream)) = tokio::time::timeout(
                    Duration::from_secs(3),
                    TcpStream::connect((address, 8080)),
                )
                .await
                {
                    break Ok::<_, crate::Error>(stream);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await??;
        tuples.push((
            link,
            udp.local_addr()?,
            udp.peer_addr()?,
            tcp.local_addr()?,
            tcp.peer_addr()?,
        ));
        write(&cli.evidence.join("original-tuples.json"), &tuples).await?;
        flows.push((link, udp, tcp));
    }
    let [external, lan] = flows.as_mut_slice() else {
        return Err("two exit service flows required".into());
    };
    tokio::try_join!(measure(cli, external), measure(cli, lan))?;
    ready(cli, "exit-window-done").await?;
    wait(cli, "exit-flow-withdraw").await?;
    for (_, udp, mut tcp) in flows {
        let mut reply = [0; 64];
        if udp.send(b"exit-revoked-old-tuple").await.is_ok()
            && tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut reply))
                .await
                .is_ok_and(|r| r.is_ok())
        {
            return Err("withdrawn exit UDP flow still replies".into());
        }
        if tcp.write_all(b"exit-revoked-old-tuple").await.is_ok()
            && tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut reply))
                .await
                .is_ok_and(|r| r.is_ok_and(|size| size > 0))
        {
            return Err("withdrawn exit TCP flow still replies".into());
        }
    }
    write(&cli.evidence.join("withdrawn-old-flows.json"), &true).await?;
    Ok(())
}

async fn measure(cli: &Cli, (link, udp, tcp): &mut (&str, UdpSocket, TcpStream)) -> Result<()> {
    let std::net::IpAddr::V4(address) = udp.peer_addr()?.ip() else {
        return Err("IPv4 exit required".into());
    };
    let full = [0x6b; 1338];
    udp.send(&full).await?;
    let skipped = expected_reply(udp, &full).await?;
    write(
        &cli.evidence.join(format!("mtu-{link}.json")),
        &(full.len(), skipped),
    )
    .await?;
    let started = std::time::Instant::now();
    let regular = async {
        let mut count = 0_u64;
        while started.elapsed() < cli.traffic_duration() {
            let mut payload = b"exit-steady-".to_vec();
            payload.extend_from_slice(&count.to_be_bytes());
            udp.send(&payload).await?;
            expected_reply(udp, &payload).await?;
            let sent = [0x5a; 16384];
            let mut echoed = [0; 16384];
            tokio::time::timeout(IO, async {
                tcp.write_all(&sent).await?;
                tcp.read_exact(&mut echoed).await
            })
            .await??;
            if echoed != sent {
                return Err("exit multi-MSS TCP reply mismatch".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            count += 1;
        }
        Ok::<_, crate::Error>(count)
    };
    let (count, receive_only) =
        tokio::try_join!(regular, receive_only(address, cli.traffic_duration()))?;
    write(
        &cli.evidence.join(format!("receive-only-{link}.json")),
        &receive_only,
    )
    .await?;
    write(
        &cli.evidence.join(format!("traffic-{link}.json")),
        &(started.elapsed().as_millis().to_string(), count),
    )
    .await?;
    Ok(())
}

async fn expected_reply(socket: &UdpSocket, expected: &[u8]) -> Result<u32> {
    tokio::time::timeout(IO, async {
        let mut received = [0; 2048];
        for skipped in 0..64 {
            let size = socket.recv(&mut received).await?;
            if &received[..size] == expected {
                return Ok(skipped);
            }
            // Initial admission can queue several distinct warmup sends before
            // its first reply. They must never satisfy a later MTU/sequence check.
            if &received[..size] != WARMUP {
                return Err(format!(
                    "exit UDP probe mismatch: expected {} bytes, received {} bytes, prefix {:?}",
                    expected.len(),
                    size,
                    &received[..size.min(24)]
                )
                .into());
            }
        }
        Err("exit warmup reply bound exceeded".into())
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queued_warmup_cannot_satisfy_the_next_udp_probe() {
        let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.connect(peer.local_addr().unwrap()).await.unwrap();
        let address = socket.local_addr().unwrap();
        peer.send_to(WARMUP, address).await.unwrap();
        peer.send_to(WARMUP, address).await.unwrap();
        peer.send_to(&[0x6b; 1338], address).await.unwrap();
        assert_eq!(expected_reply(&socket, &[0x6b; 1338]).await.unwrap(), 2);
        peer.send_to(b"exit-steady-old-sequence", address)
            .await
            .unwrap();
        assert!(
            expected_reply(&socket, b"exit-steady-current-sequence")
                .await
                .is_err()
        );
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceiveOnly {
    local: std::net::SocketAddr,
    remote: std::net::SocketAddr,
    received: u64,
    elapsed_ms: u64,
}

async fn receive_only(address: Ipv4Addr, duration: Duration) -> Result<ReceiveOnly> {
    let socket = initial_udp(address, 8081).await?;
    let mut bytes = [0; 8];
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            if socket.send(b"exit-subscribe").await.is_err() {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            if tokio::time::timeout(Duration::from_secs(1), socket.recv(&mut bytes))
                .await
                .is_ok_and(|r| r.is_ok_and(|count| count == 8))
            {
                break;
            }
        }
        Ok::<(), crate::Error>(())
    })
    .await??;
    let started = std::time::Instant::now();
    let mut prior = u64::from_be_bytes(bytes);
    let mut received = 1;
    while started.elapsed() < duration {
        let count = tokio::time::timeout(IO, socket.recv(&mut bytes)).await??;
        let sequence = u64::from_be_bytes(bytes);
        if count != 8 || sequence <= prior {
            return Err("exit receive-only sequence mismatch".into());
        }
        prior = sequence;
        received += 1;
    }
    Ok(ReceiveOnly {
        local: socket.local_addr()?,
        remote: socket.peer_addr()?,
        received,
        elapsed_ms: started.elapsed().as_millis().try_into()?,
    })
}

async fn initial_udp(address: Ipv4Addr, port: u16) -> Result<UdpSocket> {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let socket = UdpSocket::bind("0.0.0.0:0").await?;
            if socket.connect((address, port)).await.is_ok() {
                return Ok(socket);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await?
}
