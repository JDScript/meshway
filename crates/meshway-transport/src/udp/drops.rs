//! The kernel's receive-drop counter of each UDP socket, keyed by the
//! socket's inode: Linux lists it as the last column (`drops`) of
//! `/proc/self/net/udp` and `udp6` for the reading process's network
//! namespace. Read only; no descriptor is held or used.
use std::collections::BTreeMap;

/// Bytes read from one table at most; a line is about 150 B.
const MAX_TABLE: u64 = 16 * 1024 * 1024;

/// The socket inode of `socket`, the key of [`receive_drops`].
#[cfg(target_os = "linux")]
pub(crate) fn inode(socket: &impl std::os::fd::AsFd) -> Option<u64> {
    rustix::fs::fstat(socket).ok().map(|stat| stat.st_ino)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn inode(_socket: &impl std::os::fd::AsFd) -> Option<u64> {
    None
}

/// Receive drops by socket inode for every UDP socket of this network
/// namespace; `None` when the tables cannot be read or parsed (unknown, not
/// zero). An absent IPv6 table (IPv6 disabled) contributes nothing.
pub fn receive_drops() -> Option<BTreeMap<u64, u64>> {
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let mut drops = BTreeMap::new();
        for (path, required) in [("/proc/self/net/udp", true), ("/proc/self/net/udp6", false)] {
            let mut text = String::new();
            match std::fs::File::open(path) {
                Ok(file) => {
                    file.take(MAX_TABLE).read_to_string(&mut text).ok()?;
                }
                Err(_) if !required => continue,
                Err(_) => return None,
            }
            parse(&text, &mut drops)?;
        }
        Some(drops)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = MAX_TABLE;
        None
    }
}

/// One `/proc/net/udp{,6}` table: a header, then `sl local rem st tx:rx
/// tr:when retrnsmt uid timeout inode ref pointer drops` per socket.
#[cfg(any(target_os = "linux", test))]
fn parse(text: &str, drops: &mut BTreeMap<u64, u64>) -> Option<()> {
    let mut lines = text.lines();
    let header: Vec<_> = lines.next()?.split_whitespace().collect();
    if header.first() != Some(&"sl") || header.last() != Some(&"drops") {
        return None;
    }
    for line in lines {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 13 {
            return None;
        }
        let inode = fields[9].parse().ok()?;
        let count = fields[fields.len() - 1].parse().ok()?;
        drops.insert(inode, count);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "   sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode ref pointer drops
 1290: 00000000:88B8 00000000:0000 07 00000000:00000000 00:00000000 00000000     0        0 4153811 2 0000000000000000 17
 1295: 0100007F:0035 00000000:0000 07 00000000:00000000 00:00000000 00000000   101        0 23156 2 0000000000000000 0
";

    #[test]
    fn the_drops_column_is_read_by_inode_and_a_foreign_format_is_unknown() {
        let mut drops = BTreeMap::new();
        parse(TABLE, &mut drops).unwrap();
        assert_eq!(drops, BTreeMap::from([(4_153_811, 17), (23_156, 0)]));
        assert!(parse("sl local rem\n", &mut BTreeMap::new()).is_none());
        assert!(parse("", &mut BTreeMap::new()).is_none());
        let truncated = TABLE
            .replace(" 17\n", "\n")
            .replace("0000000000000000\n", "\n");
        assert!(parse(&truncated, &mut BTreeMap::new()).is_none());
    }

    /// A socket nobody reads, filled past its receive buffer: the kernel's
    /// counter for exactly that socket grows.
    #[cfg(target_os = "linux")]
    #[test]
    fn an_unread_socket_filled_past_its_buffer_counts_receive_drops() {
        use socket2::SockRef;
        let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        SockRef::from(&receiver).set_recv_buffer_size(4096).unwrap();
        let sender = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let key = inode(&receiver).unwrap();
        let before = receive_drops().unwrap().get(&key).copied().unwrap();
        for _ in 0..256 {
            sender
                .send_to(&[7; 1200], receiver.local_addr().unwrap())
                .unwrap();
        }
        let after = receive_drops().unwrap()[&key];
        assert!(after > before, "drops {before} → {after}");
    }
}
