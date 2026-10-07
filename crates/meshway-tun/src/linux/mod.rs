//! I/O and bounded buffer adaptation only. The codec is tun-rs 2.8.11
//! (Apache-2.0), whose algorithm originates in WireGuard (MIT). We call its
//! exported codec functions rather than copying that implementation. Re-audit
//! these doc-hidden exports when upgrading the pinned dependency.
use std::os::unix::fs::FileTypeExt;

use crate::{Capabilities, Error, Options, PacketBatch, Tun};
use tun_rs::{
    GROTable, VIRTIO_NET_HDR_GSO_TCPV4, VIRTIO_NET_HDR_GSO_TCPV6, VIRTIO_NET_HDR_GSO_UDP_L4,
    VIRTIO_NET_HDR_LEN, VirtioNetHdr, checksum, gso_split,
};

const MAX_SUPER: usize = 65535;
const MAX_SEGMENTS: usize = 1024;
const BATCH_SIZE: usize = 32;

#[cfg(test)]
mod tests;

pub(crate) struct ReadState {
    scratch: Vec<u8>,
    buffers: Vec<Vec<u8>>,
    sizes: Vec<usize>,
    pending: std::ops::Range<usize>,
}

#[derive(Default)]
pub(crate) struct WriteState {
    table: GROTable,
    buffers: Vec<Vec<u8>>,
}

impl WriteState {
    /// tun-rs can combine nonadjacent inputs. Reduce to the first aggregate's
    /// segment count and re-evaluate that prefix until it is exactly one write.
    /// This retains the public prefix/one-syscall contract without duplicating
    /// the upstream flow/sequence/checksum rules.
    fn prepare(&mut self, packets: &[&[u8]], udp: bool) -> Result<usize, Error> {
        let mut count = packets.len().min(BATCH_SIZE);
        // Upstream checks transport checksums when merging, but not the IPv4
        // header checksum or UDP length. Keep Meshway's stricter admission.
        let eligible = gro_admissible(packets[0]);
        if eligible {
            count = packets[..count]
                .iter()
                .take_while(|packet| gro_admissible(packet))
                .count();
        } else {
            count = 1;
        }
        if self.buffers.len() < count {
            self.buffers.resize_with(count, Vec::new);
        }
        loop {
            for (buffer, packet) in self.buffers.iter_mut().zip(&packets[..count]) {
                buffer.clear();
                // ExpandBuffer's coalescing capacity check includes 2 * offset.
                buffer.reserve(MAX_SUPER + 2 * VIRTIO_NET_HDR_LEN);
                buffer.resize(VIRTIO_NET_HDR_LEN, 0);
                buffer.extend_from_slice(packet);
            }
            if !eligible {
                return Ok(1);
            }
            self.table
                .apply_gro(&mut self.buffers[..count], VIRTIO_NET_HDR_LEN, udp)?;
            let first = &self.buffers[0];
            let header = VirtioNetHdr::decode(first)?;
            let merged = if header.gso_type == 0 {
                1
            } else {
                (first.len() - VIRTIO_NET_HDR_LEN - usize::from(header.hdr_len))
                    .div_ceil(usize::from(header.gso_size))
            };
            if merged == count || merged == 1 {
                // This is the actual number accepted when the single write
                // succeeds, returned to callers for their per-batch reports.
                return Ok(merged);
            }
            count = merged;
        }
    }
}

pub(super) fn create(options: Options) -> Result<Tun, Error> {
    tokio::runtime::Handle::try_current().map_err(|_| Error::NoRuntime)?;

    // tun-rs attempts mkdir/mknod when this node is absent. Device provisioning
    // belongs to the host, so fail before invoking its builder in that case.
    if !std::fs::metadata("/dev/net/tun")?
        .file_type()
        .is_char_device()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "/dev/net/tun must be an existing character device",
        )
        .into());
    }

    // Leaving ifr_name empty selects a fresh kernel-allocated tun%d, rather than
    // looking up a user-selected existing interface. Never call name()/persist()
    // on the builder or expose the underlying device to callers.
    let build = |offload| {
        let mut builder = tun_rs::DeviceBuilder::new()
            .layer(tun_rs::Layer::L3)
            .mtu(options.mtu)
            .offload(offload)
            .multi_queue(false)
            .packet_information(false)
            .enable(true);
        if let Some((address, prefix)) = options.ipv4 {
            builder = builder.ipv4(address, prefix, None);
        }
        if let Some((address, prefix)) = options.ipv6 {
            builder = builder.ipv6(address, prefix);
        }
        builder.build_async()
    };
    let device = if options.batch_io {
        match build(true) {
            Ok(device) if device.tcp_gso() => device,
            Ok(device) => {
                drop(device);
                build(false)?
            }
            Err(error) if matches!(error.raw_os_error(), Some(22 | 25 | 95)) => build(false)?,
            Err(error) => return Err(error.into()),
        }
    } else {
        build(false)?
    };
    let tcp = device.tcp_gso();
    let udp = device.udp_gso();
    let capabilities = Capabilities {
        vnet_header: tcp,
        checksum: tcp,
        tso4: tcp,
        tso6: tcp,
        uso4: udp,
        uso6: udp,
    };
    let name = device.name()?;
    let mtu = usize::from(device.mtu()?);
    Ok(Tun {
        device,
        name,
        mtu,
        capabilities,
        read: tokio::sync::Mutex::new(ReadState {
            scratch: vec![
                0;
                if tcp {
                    VIRTIO_NET_HDR_LEN + MAX_SUPER
                } else {
                    mtu
                }
            ],
            buffers: Vec::new(),
            sizes: Vec::new(),
            pending: 0..0,
        }),
        write: tokio::sync::Mutex::new(WriteState::default()),
    })
}

async fn fill(tun: &Tun, state: &mut ReadState, wait: bool) -> Result<(), Error> {
    if state.pending.is_empty() {
        let size = if wait {
            tun.device.recv(&mut state.scratch).await?
        } else {
            match tun.device.try_recv(&mut state.scratch) {
                Ok(size) => size,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        };
        if tun.capabilities.vnet_header {
            state.normalize(size, tun.mtu)?;
        } else {
            state.buffers.resize_with(1, Vec::new);
            state.buffers[0].clear();
            state.buffers[0].extend_from_slice(&state.scratch[..size]);
            state.sizes.resize(1, size);
            state.sizes[0] = size;
            state.pending = 0..1;
        }
    }
    Ok(())
}

fn invalid() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid TUN offload metadata",
    )
}

impl ReadState {
    /// Validate the boundary before calling the low-level upstream splitter.
    /// In particular hdr_len from Linux FORWARD paths is not authoritative.
    fn normalize(&mut self, size: usize, mtu: usize) -> Result<(), Error> {
        let mut header = VirtioNetHdr::decode(&self.scratch[..size])?;
        let input = &mut self.scratch[VIRTIO_NET_HDR_LEN..size];
        if input.is_empty() || input.len() > MAX_SUPER || header.flags & !3 != 0 {
            return Err(invalid().into());
        }
        let start = usize::from(header.csum_start);
        let checksum_at = start + usize::from(header.csum_offset);
        if header.gso_type == 0 {
            if input.len() > mtu {
                return Err(invalid().into());
            }
            if header.flags & 1 != 0 {
                if checksum_at + 2 > input.len() {
                    return Err(invalid().into());
                }
                // gso_none_checksum is not exported. Use the upstream checksum
                // primitive, including the kernel's pseudo-header seed in place.
                let value = !checksum(&input[start..], 0);
                input[checksum_at..checksum_at + 2]
                    .copy_from_slice(&if value == 0 { u16::MAX } else { value }.to_be_bytes());
            }
            self.buffers.resize_with(1, Vec::new);
            self.buffers[0].clear();
            self.buffers[0].extend_from_slice(input);
            self.sizes.resize(1, input.len());
            self.sizes[0] = input.len();
            self.pending = 0..1;
            return Ok(());
        }
        let (ip, transport, tcp, v6) = layout(input).ok_or_else(invalid)?;
        let kind = match (tcp, v6) {
            (true, false) => VIRTIO_NET_HDR_GSO_TCPV4,
            (true, true) => VIRTIO_NET_HDR_GSO_TCPV6,
            (false, _) => VIRTIO_NET_HDR_GSO_UDP_L4,
        };
        let head = ip + transport;
        let segment = usize::from(header.gso_size);
        if header.gso_type != kind
            || header.flags & 1 == 0
            || segment == 0
            || head + segment > mtu
            || start != ip
            || usize::from(header.csum_offset) != if tcp { 16 } else { 6 }
            || input.len() <= head
        {
            return Err(invalid().into());
        }
        let count = (input.len() - head).div_ceil(segment);
        if count > MAX_SEGMENTS {
            return Err(invalid().into());
        }
        header.hdr_len = head as u16;
        self.buffers.resize_with(count, Vec::new);
        self.sizes.resize(count, 0);
        for buffer in &mut self.buffers {
            buffer.resize(head + segment, 0);
        }
        let count = gso_split(input, header, &mut self.buffers, &mut self.sizes, 0, v6)?;
        for (index, buffer) in self.buffers[..count].iter_mut().enumerate() {
            // Preserve the original boundary's UDP zero-checksum representation.
            if !tcp && buffer[checksum_at..checksum_at + 2] == [0, 0] {
                buffer[checksum_at..checksum_at + 2].fill(0xff);
            }
            if tcp && index != 0 && buffer[ip + 13] & 0x80 != 0 {
                buffer[ip + 13] &= !0x80;
                buffer[checksum_at..checksum_at + 2].fill(0);
                let size = self.sizes[index];
                let addresses = if v6 { &buffer[8..40] } else { &buffer[12..20] };
                let pseudo = checksum(addresses, (size - ip + 6) as u64);
                let value = !checksum(&buffer[ip..size], u64::from(pseudo));
                buffer[checksum_at..checksum_at + 2].copy_from_slice(&value.to_be_bytes());
            }
        }
        self.pending = 0..count;
        Ok(())
    }
}

/// Structural/offset validation, shared by the read boundary and GRO admission.
/// Segmentation, aggregation and checksum algorithms remain upstream-owned.
fn layout(packet: &[u8]) -> Option<(usize, usize, bool, bool)> {
    let (ip, protocol, v6) = match packet.first()? >> 4 {
        4 if packet.len() >= 20 => {
            let ip = usize::from(packet[0] & 15) * 4;
            if ip < 20
                || ip > packet.len()
                || u16::from_be_bytes([packet[6], packet[7]]) & 0x3fff != 0
            {
                return None;
            }
            (ip, packet[9], false)
        }
        6 if packet.len() >= 40 => (40, packet[6], true),
        _ => return None,
    };
    let tcp = protocol == 6;
    let transport = match protocol {
        6 if packet.len() >= ip + 20 => usize::from(packet[ip + 12] >> 4) * 4,
        17 if packet.len() >= ip + 8 => 8,
        _ => return None,
    };
    if (tcp && transport < 20) || ip + transport > packet.len() {
        return None;
    }
    Some((ip, transport, tcp, v6))
}

fn gro_admissible(packet: &[u8]) -> bool {
    let Some((ip, _, tcp, v6)) = layout(packet) else {
        return false;
    };
    (v6 || checksum(&packet[..ip], 0) == u16::MAX)
        && (tcp
            || usize::from(u16::from_be_bytes([packet[ip + 4], packet[ip + 5]]))
                == packet.len() - ip)
        && (!tcp || packet[ip + 12] & 15 == 0)
}

pub(super) async fn recv(tun: &Tun, buffer: &mut [u8]) -> Result<usize, Error> {
    let mut state = tun.read.lock().await;
    if !tun.capabilities.vnet_header && state.pending.is_empty() {
        return Ok(tun.device.recv(buffer).await?);
    }
    fill(tun, &mut state, true).await?;
    let index = state
        .pending
        .next()
        .expect("successful read yields a packet");
    let size = state.sizes[index];
    buffer[..size].copy_from_slice(&state.buffers[index][..size]);
    Ok(size)
}

pub(super) async fn recv_batch(tun: &Tun, batch: &mut PacketBatch) -> Result<(), Error> {
    let mut state = tun.read.lock().await;
    // Reuse previously delivered allocations when the caller has not taken them.
    for packet in batch.packets.drain(..) {
        if let Some(slot) = state.buffers.iter_mut().find(|slot| slot.capacity() == 0) {
            *slot = packet;
        }
    }
    fill(tun, &mut state, true).await?;
    let mut bytes = 0;
    loop {
        if state.pending.is_empty() && batch.packets.len() < BATCH_SIZE && bytes < 65536 {
            fill(tun, &mut state, false).await?;
        }
        let Some(index) = state.pending.clone().next() else {
            break;
        };
        let size = state.sizes[index];
        if batch.packets.len() == BATCH_SIZE || bytes + size > 65536 {
            break;
        }
        bytes += size;
        state.pending.next();
        let mut packet = std::mem::take(&mut state.buffers[index]);
        packet.truncate(size);
        batch.packets.push(packet);
    }
    Ok(())
}

pub(super) async fn send_batch(tun: &Tun, packets: &[&[u8]]) -> Result<usize, Error> {
    let mut state;
    let (bytes, count) = if tun.capabilities.vnet_header {
        state = tun.write.lock().await;
        let count = state.prepare(packets, tun.capabilities.uso4)?;
        (state.buffers[0].as_slice(), count)
    } else {
        (packets[0], 1)
    };
    let actual = tun.device.send(bytes).await?;
    if actual != bytes.len() {
        return Err(Error::ShortWrite {
            expected: bytes.len(),
            actual,
        });
    }
    Ok(count)
}
