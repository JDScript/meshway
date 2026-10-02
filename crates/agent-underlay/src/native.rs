//! Existing dataplane socket traits over owning protected FDs.
use crate::{ProtectedListener, ProtectedTcp, ProtectedUdp};
use meshway_dataplane::sockets::{NativeListener, NativeTcp, NativeUdp, SocketFuture};
use std::{io, net::SocketAddr};
impl NativeTcp for ProtectedTcp {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.peer_addr()
    }
    fn set_nodelay(&self, value: bool) -> io::Result<()> {
        self.set_nodelay(value)
    }
}
impl NativeListener for ProtectedListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }
    fn accept(&self) -> SocketFuture<'_, (Box<dyn NativeTcp>, SocketAddr)> {
        Box::pin(async move {
            let (socket, address) = self.accept().await?;
            Ok((Box::new(socket) as Box<dyn NativeTcp>, address))
        })
    }
}
impl NativeUdp for ProtectedUdp {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }
    fn writable(&self) -> SocketFuture<'_, ()> {
        Box::pin(self.writable())
    }
    fn try_send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize> {
        self.try_send_to(bytes, remote)
    }
    fn recv_from<'a>(&'a self, bytes: &'a mut [u8]) -> SocketFuture<'a, (usize, SocketAddr)> {
        Box::pin(self.recv_from(bytes))
    }
    fn send_to<'a>(&'a self, bytes: &'a [u8], remote: SocketAddr) -> SocketFuture<'a, usize> {
        Box::pin(self.send_to(bytes, remote))
    }
}
