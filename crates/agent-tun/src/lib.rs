//! Owned nonpersistent TUN devices. Routing and business authorization are callers'
//! responsibilities; no controller identity, packet plan or logger lives here.
use meshway_dataplane::packet_io::{Device as PacketDevice, PacketBatch};
use std::{io, net::Ipv4Addr};

#[cfg(any(target_os = "linux", test))]
mod ipv6;
#[cfg(target_os = "linux")]
mod linux;

pub struct Configuration {
    pub name: String,
    pub mtu: u16,
    pub address: Option<Ipv4Addr>,
    pub prefix_len: u8,
    pub peer: Option<Ipv4Addr>,
}

impl Configuration {
    pub fn validate(&self) -> io::Result<()> {
        if self.name.is_empty()
            || self.name.len() > 15
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || self.mtu < 576
            || self.prefix_len > 32
            || self
                .address
                .is_some_and(|ip| ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast())
            || self
                .peer
                .is_some_and(|ip| ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast())
            || self.peer.is_some() && self.address.is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid TUN device configuration",
            ));
        }
        Ok(())
    }
}

pub struct Device {
    #[cfg(target_os = "linux")]
    inner: linux::Device,
}

/// Borrowed proof of a real owned live FD; callers cannot fabricate name/index.
pub struct Identity<'a> {
    device: &'a Device,
}

impl Device {
    pub async fn open(configuration: &Configuration) -> io::Result<Self> {
        configuration.validate()?;
        #[cfg(target_os = "linux")]
        {
            Ok(Self {
                inner: linux::Device::open(configuration).await?,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Linux TUN required",
            ))
        }
    }

    pub fn identity(&self) -> io::Result<Identity<'_>> {
        let proof = Identity { device: self };
        proof.verify()?;
        Ok(proof)
    }

    pub async fn recv(&self, bytes: &mut [u8]) -> io::Result<usize> {
        #[cfg(target_os = "linux")]
        {
            self.inner.device.recv(bytes).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = bytes;
            Err(unsupported())
        }
    }

    pub async fn send(&self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(target_os = "linux")]
        {
            self.inner.device.send(bytes).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = bytes;
            Err(unsupported())
        }
    }

    pub fn try_recv(&self, bytes: &mut [u8]) -> io::Result<usize> {
        #[cfg(target_os = "linux")]
        {
            self.inner.device.try_recv(bytes)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = bytes;
            Err(unsupported())
        }
    }
}

impl Identity<'_> {
    pub fn name(&self) -> io::Result<String> {
        self.verify()?;
        #[cfg(target_os = "linux")]
        {
            self.device.inner.device.name()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(unsupported())
        }
    }

    pub fn index(&self) -> io::Result<u32> {
        self.verify()?;
        #[cfg(target_os = "linux")]
        {
            Ok(self.device.inner.index)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(unsupported())
        }
    }

    pub fn verify(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.device.inner.verify()
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self.device;
            Err(unsupported())
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, "Linux TUN required")
}

impl PacketDevice for Device {
    async fn recv_batch(&self) -> io::Result<PacketBatch> {
        #[cfg(target_os = "linux")]
        {
            let mut bytes = self.inner.buffer.lock().await;
            loop {
                let length = self.recv(&mut bytes).await?;
                if length == 0 {
                    return Err(io::Error::other("TUN closed"));
                }
                if length > meshway_dataplane::MAX_PACKET_PAYLOAD {
                    tokio::task::yield_now().await;
                    continue;
                }
                return PacketBatch::new(vec![bytes[..length].to_vec()]);
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(unsupported())
        }
    }

    async fn send_batch(&self, packets: PacketBatch) -> io::Result<()> {
        for bytes in packets.into_packets() {
            if self.send(&bytes).await? != bytes.len() {
                return Err(io::Error::other("short TUN write"));
            }
        }
        Ok(())
    }

    fn drain(&self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner.drain()
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(unsupported())
        }
    }
}
