use std::{io, net::Ipv4Addr, sync::Arc};

use meshway_dataplane::Runtime;
use tokio::{sync::watch, time::Instant};

use super::{PacketPlan, tun::Device};

/// One complete IPv4 packet. IP options, including source-route options, are
/// deliberately unsupported. Protocol/ports do not confer or narrow pair grants.
pub(super) fn ipv4(bytes: &[u8], mtu: u16) -> Option<(Ipv4Addr, Ipv4Addr)> {
    if bytes.len() < 20
        || bytes.len() > usize::from(mtu)
        || bytes[0] != 0x45
        || usize::from(u16::from_be_bytes([bytes[2], bytes[3]])) != bytes.len()
        || u16::from_be_bytes([bytes[6], bytes[7]]) & 0xbfff != 0
        || bytes[8] == 0
    {
        return None;
    }
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    if sum != 0xffff {
        return None;
    }
    Some((
        Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]),
        Ipv4Addr::new(bytes[16], bytes[17], bytes[18], bytes[19]),
    ))
}

fn current(deadline: &watch::Receiver<Option<Instant>>) -> bool {
    deadline
        .borrow()
        .is_some_and(|until| until > Instant::now())
}

/// A single bounded duplex task, cancelled as a whole by the session watchdog.
/// No per-packet tasks or unbounded queues; dropped sends are never delivery proofs.
pub(crate) async fn run(
    device: Arc<dyn Device>,
    runtime: Arc<Runtime>,
    plan: PacketPlan,
    deadline: watch::Receiver<Option<Instant>>,
    name: &str,
) -> io::Result<()> {
    let mut received = runtime
        .take_packet_receiver()
        .map_err(|_| io::Error::other("runtime packet receiver unavailable"))?;
    if !current(&deadline) {
        return Ok(());
    }
    super::tun::proof(name, &plan, true, "generation_active");
    let outbound = async {
        let mut bytes = vec![0; 65_536];
        loop {
            let length = device.recv(&mut bytes).await?;
            if length == 0 {
                return Err(io::Error::other("TUN closed"));
            }
            if !current(&deadline) {
                return Ok(());
            }
            if let Some(path) = plan.outgoing(&bytes[..length]) {
                match runtime
                    .send_packet(path.clone(), bytes[..length].to_vec())
                    .await
                {
                    Ok(()) => {} // Local enqueue only; never logged as remote delivery.
                    Err(
                        meshway_dataplane::Error::Unavailable
                        | meshway_dataplane::Error::Capacity
                        | meshway_dataplane::Error::Timeout,
                    ) => {}
                    // Closed resources, local protocol/authentication failures
                    // and impossible plan mismatches withdraw the generation.
                    Err(_) => return Err(io::Error::other("packet runtime failure")),
                }
            }
            // Even continuously readable malformed traffic must yield to withdrawal.
            tokio::task::yield_now().await;
        }
    };
    let inbound = async {
        loop {
            let packet = received
                .recv()
                .await
                .ok_or_else(|| io::Error::other("packet runtime closed"))?;
            if !current(&deadline) {
                return Ok(());
            }
            if plan.incoming(&packet) {
                let length = device.send(&packet.payload).await?;
                if length != packet.payload.len() {
                    return Err(io::Error::other("short TUN write"));
                }
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::try_join!(outbound, inbound).map(|_| ())
}
