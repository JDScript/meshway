#![forbid(unsafe_code)]

//! Owned Linux L3 TUN packet I/O.
//!
//! Creation configures a new, nonpersistent, kernel-named interface in the
//! calling thread's current network namespace. The host supplies that namespace
//! and `/dev/net/tun`; this library never switches namespaces. Optional addresses
//! cause the kernel to install its normal local/connected routes for this device.
//! There is no route manager, DNS configuration, business policy, or forwarding here.
//!
//! A Tokio runtime with I/O enabled must be entered during creation and remain
//! alive during I/O. The device owns its descriptor, has no background tasks, and
//! is removed by the kernel when dropped. There is no adoption, persistence,
//! descriptor export, multiqueue, Ethernet, or packet-information header.

use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

mod device;
pub use device::{Io, PacketDevice};

/// Largest inner packets the path toward a destination carries, as the
/// packet owner knows them from its channels: per Mesh address of a peer,
/// and for every destination outside the Mesh. A host clamps the TCP MSS of
/// SYNs crossing a TUN to them. A missing entry needs no clamp. Sizes only;
/// no permission.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PathLimits {
    pub peers: BTreeMap<IpAddr, u16>,
    pub other: Option<u16>,
}

#[cfg(target_os = "linux")]
mod linux;

/// Startup configuration. Names are always allocated by the kernel, preventing
/// attachment to an existing named interface.
#[derive(Clone, Debug)]
pub struct Options {
    mtu: u16,
    ipv4: Option<(Ipv4Addr, u8)>,
    ipv6: Option<(Ipv6Addr, u8)>,
    batch_io: bool,
}

impl Options {
    /// Request Linux virtio-net offload, probing the kernel at device creation.
    /// The default keeps the original per-packet device mode.
    pub fn with_batch_io(mut self, enabled: bool) -> Self {
        self.batch_io = enabled;
        self
    }
    /// Set the packet MTU, in bytes, in `1280..=65535`.
    ///
    /// The lower bound permits IPv6; IPv6 jumbograms are not supported.
    pub fn new(mtu: usize) -> Result<Self, Error> {
        let mtu = u16::try_from(mtu)
            .ok()
            .filter(|mtu| *mtu >= 1280)
            .ok_or(Error::InvalidMtu { mtu })?;
        Ok(Self {
            mtu,
            ipv4: None,
            ipv6: None,
            batch_io: false,
        })
    }

    /// Assign one IPv4 address and prefix to the newly owned interface.
    /// Repeated calls replace the previous IPv4 assignment.
    pub fn with_ipv4(mut self, address: Ipv4Addr, prefix: u8) -> Result<Self, Error> {
        if prefix > 32 {
            return Err(Error::InvalidPrefix { family: 4, prefix });
        }
        self.ipv4 = Some((address, prefix));
        Ok(self)
    }

    /// Assign one IPv6 address and prefix to the newly owned interface.
    /// Repeated calls replace the previous IPv6 assignment. Kernel IPv6 address
    /// readiness (including duplicate-address detection) is not awaited.
    pub fn with_ipv6(mut self, address: Ipv6Addr, prefix: u8) -> Result<Self, Error> {
        if prefix > 128 {
            return Err(Error::InvalidPrefix { family: 6, prefix });
        }
        self.ipv6 = Some((address, prefix));
        Ok(self)
    }
}

/// Device configuration or packet I/O failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("TUN device creation is supported only on Linux")]
    Unsupported,
    #[error("MTU {mtu} is outside 1280..=65535")]
    InvalidMtu { mtu: usize },
    #[error("invalid IPv{family} prefix length {prefix}")]
    InvalidPrefix { family: u8, prefix: u8 },
    #[error("TUN creation requires an entered Tokio runtime with I/O enabled")]
    NoRuntime,
    #[error("receive buffer has {actual} bytes; at least MTU {required} is required")]
    BufferTooSmall { required: usize, actual: usize },
    #[error("packet has {actual} bytes; expected 1..={mtu}")]
    InvalidPacketSize { mtu: usize, actual: usize },
    #[error("short TUN datagram write: wrote {actual} of {expected} bytes")]
    ShortWrite { expected: usize, actual: usize },
    #[error("TUN I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// A single owned L3 TUN queue. The private descriptor is the resource identity;
/// the cached name is informational and is never used for deletion.
pub struct Tun {
    #[cfg(target_os = "linux")]
    device: tun_rs::AsyncDevice,
    #[cfg(target_os = "linux")]
    read: tokio::sync::Mutex<linux::ReadState>,
    #[cfg(target_os = "linux")]
    write: tokio::sync::Mutex<linux::WriteState>,
    capabilities: Capabilities,
    name: String,
    mtu: usize,
}

/// Actual successfully enabled kernel capabilities, not just requested flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    pub vnet_header: bool,
    pub checksum: bool,
    pub tso4: bool,
    pub tso6: bool,
    pub uso4: bool,
    pub uso6: bool,
}

/// One bounded delivery batch (32 complete packets, at most 64 KiB).
#[derive(Default)]
pub struct PacketBatch {
    packets: Vec<Vec<u8>>,
}

impl std::fmt::Debug for PacketBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PacketBatch")
            .field("packets", &self.packets.len())
            .field("bytes", &self.packets.iter().map(Vec::len).sum::<usize>())
            .finish()
    }
}

impl PacketBatch {
    /// Add one normalized packet, enforcing the shared bounded read contract.
    pub fn push(&mut self, packet: Vec<u8>) -> Result<(), Error> {
        if packet.is_empty()
            || self.packets.len() >= 32
            || self.packets.iter().map(Vec::len).sum::<usize>() + packet.len() > 65536
        {
            return Err(Error::InvalidPacketSize {
                mtu: 65536,
                actual: packet.len(),
            });
        }
        self.packets.push(packet);
        Ok(())
    }
    pub fn packets(&self) -> &[Vec<u8>] {
        &self.packets
    }
    pub fn take(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.packets)
    }
}

impl Tun {
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Receive immediately available complete packets without waiting to fill
    /// a batch. Cancellation retains already-read aggregate segments in the
    /// device's bounded receive state (one aggregate, at most 65,535 bytes and
    /// 1,024 segments; larger aggregates are errors). Only one receive owner is
    /// permitted.
    pub async fn recv_batch(&self, batch: &mut PacketBatch) -> Result<(), Error> {
        #[cfg(target_os = "linux")]
        {
            linux::recv_batch(self, batch).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = batch;
            Err(Error::Unsupported)
        }
    }

    /// Copy normalized packets directly into caller-owned storage. The callback
    /// runs synchronously, once per packet, under the same bounded receive owner
    /// as `recv_batch`. Segmentation storage remains with the device for reuse.
    /// No callback runs before a successful read; cancellation while waiting
    /// retains the device's unread normalized suffix.
    pub async fn recv_batch_map<T>(&self, map: impl FnMut(&[u8]) -> T) -> Result<Vec<T>, Error> {
        #[cfg(target_os = "linux")]
        {
            linux::recv_batch_map(self, map).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = map;
            Err(Error::Unsupported)
        }
    }

    /// Write a compatible adjacent prefix in one syscall. Returns the number
    /// written; callers continue the suffix with their original output guard.
    /// At most one syscall is attempted per call; cancellation cannot leave a
    /// background sender. Completion means local kernel acceptance only.
    pub async fn send_batch(&self, packets: &[&[u8]]) -> Result<usize, Error> {
        if packets.is_empty() {
            return Ok(0);
        }
        for packet in packets {
            if packet.is_empty() || packet.len() > self.mtu {
                return Err(Error::InvalidPacketSize {
                    mtu: self.mtu,
                    actual: packet.len(),
                });
            }
        }
        #[cfg(target_os = "linux")]
        {
            linux::send_batch(self, packets).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(Error::Unsupported)
        }
    }
    /// Synchronously create, configure, and enable a new interface.
    ///
    /// Requires an entered Tokio runtime with its I/O driver enabled, an existing
    /// `/dev/net/tun`, and network-administration permission in the current
    /// namespace. Every startup failure releases the newly owned descriptor.
    /// No caller-selected name is accepted: tun-rs does not use `IFF_TUN_EXCL`.
    ///
    /// # Panics
    ///
    /// On Linux, Tokio's `AsyncFd` panics if the entered runtime has its I/O
    /// driver disabled. An absent runtime is reported as [`Error::NoRuntime`].
    pub fn create(options: Options) -> Result<Self, Error> {
        #[cfg(target_os = "linux")]
        {
            linux::create(options)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (options.mtu, options.ipv4, options.ipv6, options.batch_io);
            Err(Error::Unsupported)
        }
    }

    /// Receive one raw IP datagram emitted by the kernel.
    ///
    /// Buffers smaller than the configured MTU are rejected before reading,
    /// preserving the queued datagram. Offloaded aggregates are normalized into
    /// complete packets; remaining segments stay in bounded device state.
    /// The host must not externally change the
    /// device's MTU or offload settings while it is owned here.
    ///
    /// Cancelling a pending receive does not consume a packet. There are no
    /// spawned workers. Offload mode retains at most one read aggregate.
    pub async fn recv(&self, buffer: &mut [u8]) -> Result<usize, Error> {
        if buffer.len() < self.mtu {
            return Err(Error::BufferTooSmall {
                required: self.mtu,
                actual: buffer.len(),
            });
        }
        #[cfg(target_os = "linux")]
        {
            linux::recv(self, buffer).await
        }
        #[cfg(not(target_os = "linux"))]
        Err(Error::Unsupported)
    }

    /// Inject exactly one raw IP datagram into the kernel.
    ///
    /// This boundary checks size, not packet syntax or policy. A successful write
    /// means kernel acceptance of the write, not application delivery. Short
    /// writes are errors and are never continued as another datagram. Cancelling
    /// a pending send does not leave a worker that can send it later.
    pub async fn send(&self, packet: &[u8]) -> Result<(), Error> {
        if packet.is_empty() || packet.len() > self.mtu {
            return Err(Error::InvalidPacketSize {
                mtu: self.mtu,
                actual: packet.len(),
            });
        }
        #[cfg(target_os = "linux")]
        {
            linux::send_batch(self, &[packet]).await?;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        Err(Error::Unsupported)
    }

    /// Kernel-assigned name at creation. External renaming is not supported.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Index of this live owned device. This is an installation reference, not
    /// an ownership token for adopting or deleting an interface after a crash.
    pub fn if_index(&self) -> Result<u32, Error> {
        #[cfg(target_os = "linux")]
        {
            Ok(self.device.if_index()?)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(Error::Unsupported)
        }
    }

    /// Configured packet MTU in bytes.
    pub fn mtu(&self) -> usize {
        self.mtu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mtu_and_prefix_validation() {
        for mtu in [0, 1, 1279, 65536, usize::MAX] {
            assert!(matches!(Options::new(mtu), Err(Error::InvalidMtu { .. })));
        }
        for mtu in [1280, 1500, 65535] {
            assert!(Options::new(mtu).is_ok());
        }
        assert!(matches!(
            Options::new(1500)
                .unwrap()
                .with_ipv4(Ipv4Addr::LOCALHOST, 33),
            Err(Error::InvalidPrefix {
                family: 4,
                prefix: 33
            })
        ));
        assert!(matches!(
            Options::new(1500)
                .unwrap()
                .with_ipv6(Ipv6Addr::LOCALHOST, 129),
            Err(Error::InvalidPrefix {
                family: 6,
                prefix: 129
            })
        ));
        for prefix in [0, 32] {
            assert!(
                Options::new(1500)
                    .unwrap()
                    .with_ipv4(Ipv4Addr::LOCALHOST, prefix)
                    .is_ok()
            );
        }
        for prefix in [0, 128] {
            assert!(
                Options::new(1500)
                    .unwrap()
                    .with_ipv6(Ipv6Addr::LOCALHOST, prefix)
                    .is_ok()
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn unsupported_without_native_runtime() {
        assert!(matches!(
            Tun::create(Options::new(1500).unwrap()),
            Err(Error::Unsupported)
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn missing_runtime_is_reported_before_opening_device() {
        assert!(matches!(
            Tun::create(Options::new(1500).unwrap()),
            Err(Error::NoRuntime)
        ));
    }
}
