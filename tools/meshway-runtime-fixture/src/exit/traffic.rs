use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
};

pub(super) async fn run(cli: &Cli, topology: &Topology) -> Result<()> {
    let mut tuples = Vec::new();
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
        let mut tcp =
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
        let started = std::time::Instant::now();
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
        write(
            &cli.evidence.join(format!("traffic-{link}.json")),
            &(started.elapsed().as_millis().to_string(), count),
        )
        .await?;
    }
    Ok(())
}
