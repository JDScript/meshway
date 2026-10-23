//! Safe packet I/O over an owned NetworkExtension utun descriptor.
use crate::{Error, Io, Options, PacketBatch, PacketDevice};
use std::{io, os::fd::OwnedFd};
use tokio::{io::unix::AsyncFd, sync::Mutex};

// Darwin's utun protocol is a network-byte-order address family followed by
// exactly one IP packet. These are Darwin AF values, not IP version numbers.
const AF_INET: u32 = 2;
const AF_INET6: u32 = 30;
const HEADER: usize = 4;

pub struct Utun {
    fd: AsyncFd<OwnedFd>,
    mtu: usize,
    read: Mutex<Vec<u8>>,
}

impl Utun {
    /// Take an exclusively used, owned utun descriptor from the NE host.
    /// The host configures its MTU and addresses through NE, and must stop all
    /// NEPacketTunnelFlow reads before handing it here. A duplicate descriptor
    /// is permitted only if its original is never used for packet I/O.
    /// No unsafe raw-descriptor adoption occurs in this library.
    pub fn from_owned_fd(fd: OwnedFd, mtu: usize) -> Result<Self, Error> {
        Options::new(mtu)?;
        tokio::runtime::Handle::try_current().map_err(|_| Error::NoRuntime)?;
        let flags = rustix::fs::fcntl_getfl(&fd).map_err(io::Error::from)?;
        rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(io::Error::from)?;
        // RUSTSEC-2026-0386: the AsyncFd owns this fd for life; the host never uses or
        // closes a duplicate of it for packet I/O (see above).
        #[allow(deprecated)]
        let fd = AsyncFd::new(fd)?;
        Ok(Self {
            fd,
            mtu,
            // Read the largest legal datagram, even if the host violated its
            // MTU contract. Never accept a silently truncated packet.
            read: Mutex::new(vec![0; usize::from(u16::MAX) + HEADER + 1]),
        })
    }

    async fn receive(&self, map: &mut (dyn FnMut(&[u8]) + Send)) -> Result<(), Error> {
        let mut buffer = self.read.lock().await;
        let mut ready = self.fd.readable().await?;
        let mut count = 0;
        let mut bytes = 0;
        loop {
            match ready.try_io(|fd| {
                rustix::io::read(fd.get_ref(), buffer.as_mut_slice()).map_err(io::Error::from)
            }) {
                Ok(Ok(size)) => {
                    let packet = decode(&buffer[..size], self.mtu)?;
                    bytes += packet.len();
                    map(packet);
                    count += 1;
                    // Reserve room for the next largest packet before reading.
                    if count == 32 || bytes + self.mtu > 65536 {
                        return Ok(());
                    }
                }
                Ok(Err(error)) => return Err(error.into()),
                Err(_) if count != 0 => return Ok(()),
                Err(_) => ready = self.fd.readable().await?,
            }
        }
    }
}

fn family(packet: &[u8]) -> Result<[u8; HEADER], Error> {
    match packet.first().map(|byte| byte >> 4) {
        Some(4) => Ok(AF_INET.to_be_bytes()),
        Some(6) => Ok(AF_INET6.to_be_bytes()),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid utun IP version").into()),
    }
}

fn decode(bytes: &[u8], mtu: usize) -> Result<&[u8], Error> {
    let packet = bytes.get(HEADER..).filter(|packet| !packet.is_empty());
    let Some(packet) = packet else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "short utun datagram").into());
    };
    if packet.len() > mtu {
        return Err(Error::InvalidPacketSize {
            mtu,
            actual: packet.len(),
        });
    }
    if bytes[..HEADER] != family(packet)? {
        return Err(
            io::Error::new(io::ErrorKind::InvalidData, "utun address family mismatch").into(),
        );
    }
    Ok(packet)
}

impl PacketDevice for Utun {
    fn mtu(&self) -> usize {
        self.mtu
    }

    fn recv_batch<'a>(&'a self, batch: &'a mut PacketBatch) -> Io<'a, ()> {
        Box::pin(async move {
            batch.take();
            self.receive(&mut |packet| {
                // receive enforces exactly the same packet/byte bounds.
                batch.push(packet.to_vec()).expect("bounded utun batch");
            })
            .await
        })
    }

    fn recv_batch_map<'a>(&'a self, map: &'a mut (dyn FnMut(&[u8]) + Send)) -> Io<'a, ()> {
        Box::pin(self.receive(map))
    }

    fn send_batch<'a>(&'a self, packets: &'a [&'a [u8]]) -> Io<'a, usize> {
        Box::pin(async move {
            let Some(packet) = packets.first() else {
                return Ok(0);
            };
            if packet.is_empty() || packet.len() > self.mtu {
                return Err(Error::InvalidPacketSize {
                    mtu: self.mtu,
                    actual: packet.len(),
                });
            }
            let header = family(packet)?;
            let buffers = [io::IoSlice::new(&header), io::IoSlice::new(packet)];
            // Exactly one syscall commits one packet. The shared carrier
            // rechecks its output guard before polling the remaining suffix.
            loop {
                let mut ready = self.fd.writable().await?;
                match ready.try_io(|fd| {
                    rustix::io::writev(fd.get_ref(), &buffers).map_err(io::Error::from)
                }) {
                    Ok(result) => {
                        let actual = result?;
                        let expected = HEADER + packet.len();
                        return if actual == expected {
                            Ok(1)
                        } else {
                            Err(Error::ShortWrite { expected, actual })
                        };
                    }
                    Err(_) => continue,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests;
