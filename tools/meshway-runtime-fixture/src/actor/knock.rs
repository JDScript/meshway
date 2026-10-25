//! L1: what a scanner sends to a public listener does not reach the
//! connectivity Manager. During the persistent window A connects to R's and
//! B's listener port the way scanners and broken clients do (reset at once,
//! junk then close, connect and stay silent) or sends junk datagrams; no
//! Node may then have stopped its Manager or withdrawn its packet plane.
use super::*;
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;

/// What A sent, per kind. Connections refused (no listener there) are not
/// counted as sent.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Knocks {
    pub resets: u64,
    pub junk: u64,
    pub silent: u64,
    pub datagrams: u64,
    pub refused: u64,
}

/// The listener port every runtime fixture Node uses.
const PORT: u16 = 35000;
/// Silent connections held at once; each for [`SILENT_HOLD`].
const SILENT: usize = 4;
const SILENT_HOLD: Duration = Duration::from_secs(3);

/// Rows whose actors A reaches on the underlay directly.
pub(super) fn enabled(cli: &Cli) -> bool {
    cli.traversal == crate::TraversalMode::Off
        && !cli.traversal_fallback
        && cli.netmap_scenario.is_none()
}

/// Knock on R's and B's listener port until `deadline`.
pub(super) async fn run(cli: &Cli, deadline: Instant) -> Result<Knocks> {
    let underlay = signals::underlay(cli).await?;
    let targets: Vec<SocketAddr> = [Role::R, Role::B]
        .iter()
        .map(|role| SocketAddr::new(underlay.actors[role].into(), PORT))
        .collect();
    let stream = !matches!(cli.transport, crate::CarrierMode::NativeUdp);
    let mut knocks = Knocks::default();
    let mut silent: Vec<(Instant, tokio::net::TcpStream)> = Vec::new();
    let mut round: u64 = 0;
    while Instant::now() < deadline {
        round += 1;
        silent.retain(|(since, _)| since.elapsed() < SILENT_HOLD);
        for target in &targets {
            if !stream {
                let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
                for index in 0..16u64 {
                    let junk = junk(round * 16 + index, 1 + (index as usize * 75) % 1200);
                    socket.send_to(&junk, target).await?;
                    knocks.datagrams += 1;
                }
                continue;
            }
            for index in 0..16u64 {
                let Some(connection) = connect(*target, &mut knocks).await? else {
                    continue;
                };
                if index < 12 {
                    // Close with RST before the listener can accept it.
                    connection.set_zero_linger()?;
                    drop(connection);
                    knocks.resets += 1;
                } else {
                    let mut connection = connection;
                    let _ = connection
                        .write_all(&junk(round * 16 + index, 64 + index as usize * 16))
                        .await;
                    drop(connection);
                    knocks.junk += 1;
                }
            }
            if silent.len() < SILENT
                && let Some(connection) = connect(*target, &mut knocks).await?
            {
                silent.push((Instant::now(), connection));
                knocks.silent += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(knocks)
}

async fn connect(target: SocketAddr, knocks: &mut Knocks) -> Result<Option<tokio::net::TcpStream>> {
    let socket = tokio::net::TcpSocket::new_v4()?;
    match tokio::time::timeout(Duration::from_secs(1), socket.connect(target)).await {
        Ok(Ok(connection)) => Ok(Some(connection)),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
            knocks.refused += 1;
            Ok(None)
        }
        Ok(Err(error)) => Err(format!("knock on {target} failed: {error}").into()),
        Err(_) => Err(format!("knock on {target} timed out").into()),
    }
}

/// Deterministic bytes no carrier accepts.
fn junk(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// This Node's own daemon log: its Manager never stopped on its own and
/// its packet plane was never withdrawn for a failed component.
pub(super) async fn verify_log(cli: &Cli) -> Result<()> {
    for event in log_events(cli).await? {
        let message = event
            .pointer("/fields/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if message.starts_with("Connectivity owner ended unrequested")
            || message.starts_with("Required packet plane component failed")
        {
            return Err(format!("listener knock reached the packet plane: {event}").into());
        }
    }
    Ok(())
}
