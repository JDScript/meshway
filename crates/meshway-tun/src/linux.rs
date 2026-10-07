use std::os::unix::fs::FileTypeExt;

use crate::{Capabilities, Error, Options, PacketBatch, Tun, offload};
use std::collections::VecDeque;

pub(crate) struct ReadState {
    scratch: Vec<u8>,
    pending: VecDeque<Vec<u8>>,
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
                    offload::HEADER + offload::MAX_SUPER
                } else {
                    mtu
                }
            ],
            pending: VecDeque::new(),
        }),
    })
}

async fn fill(tun: &Tun, state: &mut ReadState) -> Result<(), Error> {
    if state.pending.is_empty() {
        let size = tun.device.recv(&mut state.scratch).await?;
        if tun.capabilities.vnet_header {
            state
                .pending
                .extend(offload::split(&state.scratch[..size], tun.mtu)?);
        } else {
            state.pending.push_back(state.scratch[..size].to_vec());
        }
    }
    Ok(())
}

pub(super) async fn recv(tun: &Tun, buffer: &mut [u8]) -> Result<usize, Error> {
    if !tun.capabilities.vnet_header {
        if let Some(packet) = tun.read.lock().await.pending.pop_front() {
            buffer[..packet.len()].copy_from_slice(&packet);
            return Ok(packet.len());
        }
        return Ok(tun.device.recv(buffer).await?);
    }
    let mut state = tun.read.lock().await;
    fill(tun, &mut state).await?;
    let packet = state
        .pending
        .pop_front()
        .expect("successful read yields a packet");
    buffer[..packet.len()].copy_from_slice(&packet);
    Ok(packet.len())
}

pub(super) async fn recv_batch(tun: &Tun, batch: &mut PacketBatch) -> Result<(), Error> {
    let mut state = tun.read.lock().await;
    fill(tun, &mut state).await?;
    batch.packets.clear();
    let mut bytes = 0;
    loop {
        if state.pending.is_empty() && batch.packets.len() < 32 && bytes < 65536 {
            match tun.device.try_recv(&mut state.scratch) {
                Ok(size) => {
                    let packets = if tun.capabilities.vnet_header {
                        offload::split(&state.scratch[..size], tun.mtu)?
                    } else {
                        vec![state.scratch[..size].to_vec()]
                    };
                    state.pending.extend(packets);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        let Some(packet) = state.pending.front() else {
            break;
        };
        if batch.packets.len() == 32 || bytes + packet.len() > 65536 {
            break;
        }
        bytes += packet.len();
        batch
            .packets
            .push(state.pending.pop_front().expect("front exists"));
    }
    Ok(())
}

pub(super) async fn send_batch(tun: &Tun, packets: &[&[u8]]) -> Result<usize, Error> {
    let merged;
    let (bytes, count) = if tun.capabilities.vnet_header {
        merged = offload::merge(&packets[..packets.len().min(32)], tun.capabilities.uso4);
        (merged.0.as_slice(), merged.1)
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
