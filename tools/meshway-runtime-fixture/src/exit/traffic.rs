use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

pub(super) async fn run(cli: &Cli, topology: &Topology) -> Result<()> {
    let mut tuples = Vec::new();
    let mut flows = Vec::new();
    for link in ["external", "lan"] {
        let address = topology.addresses[link][link];
        let udp = UdpSocket::bind("0.0.0.0:0").await?;
        udp.connect((address, 8080)).await?;
        let payload = b"meshway-exit-encrypted-through-r";
        let mut received = [0; 2048];
        // Demand creation may hold/drop the initial packet. Retry only initial
        // admission; steady traffic and all later stages have exact deadlines.
        tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                udp.send(payload).await?;
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
        let tcp =
            tokio::time::timeout(Duration::from_secs(45), TcpStream::connect((address, 8080)))
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
        udp.send(b"exit-revoked-old-tuple").await?;
        let mut reply = [0; 64];
        if tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut reply))
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
    let payload = b"meshway-exit-encrypted-through-r";
    let mut received = [0; 2048];
    let full = [0x6b; 1338];
    udp.send(&full).await?;
    let size = tokio::time::timeout(IO, udp.recv(&mut received)).await??;
    if received[..size] != full {
        return Err("exit full-MTU UDP mismatch".into());
    }
    let started = std::time::Instant::now();
    let regular = async {
        let mut count = 0_u64;
        while started.elapsed() < cli.traffic_duration() {
            udp.send(payload).await?;
            let received_count = tokio::time::timeout(IO, udp.recv(&mut received)).await??;
            if &received[..received_count] != payload {
                return Err("exit UDP reply mismatch".into());
            }
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceiveOnly {
    local: std::net::SocketAddr,
    remote: std::net::SocketAddr,
    received: u64,
    elapsed_ms: u64,
}

async fn receive_only(address: Ipv4Addr, duration: Duration) -> Result<ReceiveOnly> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect((address, 8081)).await?;
    let mut bytes = [0; 8];
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            socket.send(b"exit-subscribe").await?;
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
