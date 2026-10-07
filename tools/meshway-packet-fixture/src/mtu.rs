//! Shared OS-socket boundary probes for library and real-daemon conformance.
use crate::Error;
use std::time::Duration;
use tokio::net::UdpSocket;

pub const INNER_MTU: usize = meshway_dataplane::secure::DEFAULT_INNER_MTU;
pub const TCP_BYTES: usize = 16 * 1024;

/// Connected UDP with the kernel's default PMTU policy. A successful oversized
/// send may emit fragments, which Meshway must refuse rather than reassemble.
pub async fn udp(socket: &UdpSocket, sentinel: &[u8]) -> Result<(), Error> {
    let ipv4 = socket.peer_addr()?.is_ipv4();
    let payload_bytes = INNER_MTU - if ipv4 { 28 } else { 48 };
    let mut payload = vec![0x5a; payload_bytes];
    getrandom::fill(&mut payload).map_err(|_| "MTU challenge entropy unavailable")?;
    payload[..sentinel.len()].copy_from_slice(sentinel);
    let initial_attempts = exchange(socket, &payload).await?;
    payload.push(0x6b);
    let oversized = match socket.send(&payload).await {
        // Linux EMSGSIZE: the local route MTU rejected the datagram.
        Err(error) if error.raw_os_error() == Some(90) => "kernel-emsgsize",
        Err(error) => return Err(error.into()),
        Ok(length) if length == payload.len() => {
            let mut response = vec![0; INNER_MTU + 1];
            let receive = async {
                loop {
                    let received = socket.recv(&mut response).await?;
                    // A delayed full-MTU retry reply is not an oversized echo.
                    if response[..received] != payload[..payload_bytes] {
                        return Ok::<_, std::io::Error>(());
                    }
                }
            };
            match tokio::time::timeout(Duration::from_secs(1), receive).await {
                Err(_) => "fragment-not-delivered",
                Ok(Err(error)) if error.raw_os_error() == Some(90) => "kernel-emsgsize",
                Ok(_) => {
                    return Err("MTU+1 UDP unexpectedly delivered or failed ambiguously".into());
                }
            }
        }
        Ok(_) => return Err("short MTU+1 UDP send".into()),
    };
    // Rejecting fragments must not stop the owner or break this original tuple.
    payload.pop();
    payload[payload_bytes - 1] ^= 1;
    let recovery_attempts = exchange(socket, &payload).await?;
    tracing::info!(
        phase = "mtu-conformance",
        family = if ipv4 { "ipv4" } else { "ipv6" },
        inner_mtu = INNER_MTU,
        payload_bytes,
        oversized,
        initial_attempts,
        recovery_attempts,
        "full-MTU UDP echo and MTU+1 rejection verified"
    );
    Ok(())
}

async fn exchange(socket: &UdpSocket, payload: &[u8]) -> Result<u32, Error> {
    let mut response = vec![0; INNER_MTU + 1];
    // Publication can legitimately cancel an accepted old datagram. Match the
    // runtime driver's bounded UDP retries; stale warmup echoes cannot satisfy
    // this fresh challenge, and every retry retains the original socket.
    for attempt in 1..=20 {
        if socket.send(payload).await? != payload.len() {
            return Err("short full-MTU UDP send".into());
        }
        let receive = async {
            loop {
                let received = socket.recv(&mut response).await?;
                if &response[..received] == payload {
                    return Ok::<_, Error>(());
                }
            }
        };
        if let Ok(result) = tokio::time::timeout(Duration::from_millis(500), receive).await {
            result?;
            return Ok(attempt);
        }
    }
    Err("full-MTU UDP echo unavailable".into())
}
