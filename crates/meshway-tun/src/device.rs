//! Platform-neutral L3 packet I/O. No authority or background packet sender.
use crate::{Error, PacketBatch, Tun};
use std::{future::Future, pin::Pin};

pub type Io<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

/// One bounded packet queue owned by the runtime.
pub trait PacketDevice: Send + Sync {
    fn mtu(&self) -> usize;
    /// Wait for a packet, then drain only immediately ready packets (at most
    /// 32 packets / 64 KiB). Cancellation retains any already-read suffix in
    /// bounded device storage; it must never leave a read-ahead task.
    fn recv_batch<'a>(&'a self, batch: &'a mut PacketBatch) -> Io<'a, ()>;
    /// Synchronously copy each normalized packet into caller-owned storage,
    /// under the same bounded read owner. This is not a queued callback.
    fn recv_batch_map<'a>(&'a self, map: &'a mut (dyn FnMut(&[u8]) + Send)) -> Io<'a, ()> {
        Box::pin(async move {
            let mut batch = PacketBatch::default();
            self.recv_batch(&mut batch).await?;
            for packet in batch.packets() {
                map(packet);
            }
            Ok(())
        })
    }
    /// Commit one adjacent prefix, returning its length. Never report an error
    /// after committing a prefix; the caller must know which suffix remains.
    /// Cancellation cannot leave a sender that writes after this future ends.
    fn send_batch<'a>(&'a self, packets: &'a [&'a [u8]]) -> Io<'a, usize>;
    fn send<'a>(&'a self, packet: &'a [u8]) -> Io<'a, ()> {
        Box::pin(async move {
            match self.send_batch(&[packet]).await? {
                1 => Ok(()),
                _ => Err(Error::ShortWrite {
                    expected: packet.len(),
                    actual: 0,
                }),
            }
        })
    }
}

impl PacketDevice for Tun {
    fn mtu(&self) -> usize {
        Tun::mtu(self)
    }
    fn recv_batch<'a>(&'a self, batch: &'a mut PacketBatch) -> Io<'a, ()> {
        Box::pin(Tun::recv_batch(self, batch))
    }
    fn recv_batch_map<'a>(&'a self, map: &'a mut (dyn FnMut(&[u8]) + Send)) -> Io<'a, ()> {
        Box::pin(async move { Tun::recv_batch_map(self, map).await.map(|_| ()) })
    }
    fn send_batch<'a>(&'a self, packets: &'a [&'a [u8]]) -> Io<'a, usize> {
        Box::pin(Tun::send_batch(self, packets))
    }
    fn send<'a>(&'a self, packet: &'a [u8]) -> Io<'a, ()> {
        Box::pin(Tun::send(self, packet))
    }
}
