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
    let mut tasks = JoinSet::new();
    let mut visits = Vec::new();
    let mut buffer = [0; 65536];
    let result = async {
        loop {
            if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? { break; }
            tokio::select! {
                accepted = tcp.accept(), if tasks.len() < 32 => {
                    let (mut stream, peer) = accepted?;
                    visits.push(Visit { protocol: "tcp", peer, local: stream.local_addr()?, bytes: 0 });
                    tasks.spawn(async move {
                        let mut buffer = [0; 32768];
                        loop {
                            let count = tokio::time::timeout(Duration::from_secs(150), stream.read(&mut buffer)).await??;
                            if count == 0 { break; }
                            tokio::time::timeout(IO, stream.write_all(&buffer[..count])).await??;
                        }
                        Ok::<(), crate::Error>(())
                    });
                }
                packet = udp.recv_from(&mut buffer) => {
                    let (count, peer) = packet?;
                    visits.push(Visit { protocol: "udp", peer, local: udp.local_addr()?, bytes: count });
                    udp.send_to(&buffer[..count], peer).await?;
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
