//! The owner's packet pool and its bounded TUN read turn.
use super::*;

/// Covers the existing 128 queued packets, 32 sends and one 32-packet read
/// turn without allocating overflow buffers; every buffer holds one TUN MTU
/// packet. The pool belongs to this owner.
pub(super) fn packet_pool(mtu: usize) -> Result<PacketPool, Problem> {
    PacketPool::new(256, mtu, DEFAULT_HEADROOM, DEFAULT_TAILROOM).map_err(|_| Problem::Tun)
}

/// A cancellation-safe TUN read has no read-ahead owner. After the first packet,
/// drain only immediately-ready reads, bounded to 32; lifecycle work then regains
/// priority. Packet admission still happens individually in the unique engine.
pub(super) async fn receive_packets(
    tun: &meshway_tun::Tun,
    buffers: &PacketPool,
) -> Result<Vec<Result<Packet, meshway_dataplane::PacketError>>, meshway_tun::Error> {
    tun.recv_batch_map(|bytes| {
        let mut buffer = buffers
            .acquire(bytes.len())
            .map_err(|_| meshway_tun::Error::Io(std::io::Error::other("packet pool exhausted")))?;
        buffer.copy_from_slice(bytes);
        Ok(Packet::parse(buffer))
    })
    .await?
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::packet_pool;

    #[test]
    fn packet_pool_holds_one_full_mtu_packet() {
        for mtu in [1280, meshway_dataplane::secure::DEFAULT_INNER_MTU, 1500] {
            let pool = packet_pool(mtu).unwrap();
            assert!(pool.acquire(mtu).is_ok(), "mtu {mtu}");
            assert!(pool.acquire(mtu + 1).is_err(), "mtu {mtu}");
        }
    }
}
