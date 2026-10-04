#![forbid(unsafe_code)]

//! Owned Linux L3 TUN packet I/O.
//!
//! Creation configures a new, nonpersistent, kernel-named interface in the
//! calling thread's current network namespace. The host supplies that namespace
//! and `/dev/net/tun`; this library never switches namespaces. Optional addresses
//! cause the kernel to install its normal local/connected routes for this device.
//! There is no route manager, DNS configuration, packet codec, or forwarding here.
//!
//! A Tokio runtime with I/O enabled must be entered during creation and remain
//! alive during I/O. The device owns its descriptor, has no background tasks, and
//! is removed by the kernel when dropped. There is no adoption, persistence,
//! descriptor export, multiqueue, offload, Ethernet, or packet-information header.

use std::net::{Ipv4Addr, Ipv6Addr};

#[cfg(target_os = "linux")]
mod linux;

/// Startup configuration. Names are always allocated by the kernel, preventing
/// attachment to an existing named interface.
#[derive(Clone, Debug)]
pub struct Options {
    mtu: u16,
    ipv4: Option<(Ipv4Addr, u8)>,
    ipv6: Option<(Ipv6Addr, u8)>,
}

impl Options {
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
    name: String,
    mtu: usize,
}

impl Tun {
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
            let _ = (options.mtu, options.ipv4, options.ipv6);
            Err(Error::Unsupported)
        }
    }

    /// Receive one raw IP datagram emitted by the kernel.
    ///
    /// Buffers smaller than the configured MTU are rejected before reading,
    /// preserving the queued datagram. With offload disabled the kernel emits
    /// packets bounded by that MTU. The host must not externally change the
    /// device's MTU or offload settings while it is owned here.
    ///
    /// Cancelling a pending receive does not consume a packet. There are no
    /// internal read-ahead buffers or spawned workers.
    pub async fn recv(&self, buffer: &mut [u8]) -> Result<usize, Error> {
        if buffer.len() < self.mtu {
            return Err(Error::BufferTooSmall {
                required: self.mtu,
                actual: buffer.len(),
            });
        }
        #[cfg(target_os = "linux")]
        {
            Ok(self.device.recv(buffer).await?)
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
            let actual = self.device.send(packet).await?;
            if actual != packet.len() {
                return Err(Error::ShortWrite {
                    expected: packet.len(),
                    actual,
                });
            }
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
