//! Owning native carrier boundary. Implementations retain socket-generation
//! fences below every read/write; this is separate from the packet fence.
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

pub type SocketFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;

/// Trusted Runtime metadata, never a business request for an exemption.
#[derive(Clone, Copy, Debug)]
pub enum NativeSocketPurpose<'a> {
    Listener { id: Option<&'a str> },
    Dial { id: Option<&'a str>, peer: &'a str },
}

pub trait NativeTcp: crate::DuplexStream + std::any::Any {
    fn peer_addr(&self) -> io::Result<SocketAddr>;
    fn set_nodelay(&self, value: bool) -> io::Result<()>;
}

pub trait NativeListener: Send + Sync {
    fn local_addr(&self) -> io::Result<SocketAddr>;
    fn accept(&self) -> SocketFuture<'_, (Box<dyn NativeTcp>, SocketAddr)>;
}

pub trait NativeUdp: Send + Sync {
    fn local_addr(&self) -> io::Result<SocketAddr>;
    fn writable(&self) -> SocketFuture<'_, ()>;
    fn try_send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize>;
    fn recv_from<'a>(&'a self, bytes: &'a mut [u8]) -> SocketFuture<'a, (usize, SocketAddr)>;
    fn send_to<'a>(&'a self, bytes: &'a [u8], remote: SocketAddr) -> SocketFuture<'a, usize>;
}

/// No raw descriptor/into_inner seam and no automatic fallback on failure.
/// Management resolves carrier names before supplying numeric endpoints.
pub trait NativeSocketFactory: Send + Sync {
    fn check_active(&self) -> io::Result<()> {
        Ok(())
    }
    fn udp(
        &self,
        bind: SocketAddr,
        receive_buffer_bytes: usize,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>>;
    fn listen(
        &self,
        bind: SocketAddr,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>>;
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        purpose: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>>;
}

/// Compatibility default: system sockets, explicitly unmarked.
pub struct SystemSocketFactory;
impl NativeSocketFactory for SystemSocketFactory {
    fn udp(
        &self,
        bind: SocketAddr,
        receive_buffer_bytes: usize,
        _: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>> {
        super::udp::bind(bind, receive_buffer_bytes)
            .map(|s| Box::new(s) as Box<dyn NativeUdp>)
            .map_err(io::Error::other)
    }
    fn listen(
        &self,
        bind: SocketAddr,
        _: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>> {
        let socket = if bind.is_ipv4() {
            tokio::net::TcpSocket::new_v4()?
        } else {
            tokio::net::TcpSocket::new_v6()?
        };
        socket.set_reuseaddr(true)?;
        socket.bind(bind)?;
        Ok(Box::new(socket.listen(1024)?))
    }
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        _: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>> {
        Box::pin(
            async move { Ok(Box::new(TcpStream::connect(endpoint).await?) as Box<dyn NativeTcp>) },
        )
    }
}
impl NativeTcp for TcpStream {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.peer_addr()
    }
    fn set_nodelay(&self, value: bool) -> io::Result<()> {
        self.set_nodelay(value)
    }
}
impl NativeListener for TcpListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }
    fn accept(&self) -> SocketFuture<'_, (Box<dyn NativeTcp>, SocketAddr)> {
        Box::pin(async move {
            let (s, a) = self.accept().await?;
            Ok((Box::new(s) as Box<dyn NativeTcp>, a))
        })
    }
}
impl NativeUdp for UdpSocket {
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

// Socket lifecycle fencing is deliberately a different instance/owner from
// Inner::packet_generation. Rebinding sockets must not revive retired packets.
pub(super) struct SocketGeneration(pub Arc<super::generation::Fence>);
impl SocketGeneration {
    pub fn new() -> Self {
        Self(super::generation::Fence::new())
    }
    pub fn retire(&self) {
        self.0.retire();
    }
    pub fn factory(&self, factory: Arc<dyn NativeSocketFactory>) -> Arc<dyn NativeSocketFactory> {
        Arc::new(GenerationFactory {
            fence: self.0.clone(),
            factory,
        })
    }
}
impl Drop for SocketGeneration {
    fn drop(&mut self) {
        self.retire();
    }
}

fn retired() -> io::Error {
    io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "native socket generation retired",
    )
}
async fn run<T>(fence: &super::generation::Fence, operation: SocketFuture<'_, T>) -> io::Result<T> {
    let mut operation = operation;
    tokio::select! {
        biased;
        _ = fence.cancelled() => Err(retired()),
        result = std::future::poll_fn(|cx| {
            let Ok(_guard) = fence.enter() else { return Poll::Ready(Err(retired())); };
            operation.as_mut().poll(cx)
        }) => result,
    }
}
struct GenerationFactory {
    fence: Arc<super::generation::Fence>,
    factory: Arc<dyn NativeSocketFactory>,
}
impl NativeSocketFactory for GenerationFactory {
    fn check_active(&self) -> io::Result<()> {
        let _guard = self.fence.enter().map_err(|_| retired())?;
        self.factory.check_active()
    }
    fn udp(
        &self,
        bind: SocketAddr,
        buffer: usize,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>> {
        let _guard = self.fence.enter().map_err(|_| retired())?;
        Ok(Box::new(GenerationUdp {
            socket: self.factory.udp(bind, buffer, purpose)?,
            fence: self.fence.clone(),
        }))
    }
    fn listen(
        &self,
        bind: SocketAddr,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>> {
        let _guard = self.fence.enter().map_err(|_| retired())?;
        Ok(Box::new(GenerationListener {
            socket: self.factory.listen(bind, purpose)?,
            fence: self.fence.clone(),
        }))
    }
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        purpose: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>> {
        Box::pin(async move {
            let socket = run(&self.fence, self.factory.dial(endpoint, purpose)).await?;
            let _guard = self.fence.enter().map_err(|_| retired())?;
            Ok(Box::new(GenerationTcp::new(socket, self.fence.clone())) as Box<dyn NativeTcp>)
        })
    }
}
struct GenerationUdp {
    socket: Box<dyn NativeUdp>,
    fence: Arc<super::generation::Fence>,
}
impl NativeUdp for GenerationUdp {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    fn writable(&self) -> SocketFuture<'_, ()> {
        Box::pin(run(&self.fence, self.socket.writable()))
    }
    fn try_send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize> {
        let _guard = self.fence.enter().map_err(|_| retired())?;
        self.socket.try_send_to(bytes, remote)
    }
    fn recv_from<'a>(&'a self, bytes: &'a mut [u8]) -> SocketFuture<'a, (usize, SocketAddr)> {
        Box::pin(run(&self.fence, self.socket.recv_from(bytes)))
    }
    fn send_to<'a>(&'a self, bytes: &'a [u8], remote: SocketAddr) -> SocketFuture<'a, usize> {
        Box::pin(run(&self.fence, self.socket.send_to(bytes, remote)))
    }
}
struct GenerationListener {
    socket: Box<dyn NativeListener>,
    fence: Arc<super::generation::Fence>,
}
impl NativeListener for GenerationListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }
    fn accept(&self) -> SocketFuture<'_, (Box<dyn NativeTcp>, SocketAddr)> {
        Box::pin(async move {
            let (socket, address) = run(&self.fence, self.socket.accept()).await?;
            let _guard = self.fence.enter().map_err(|_| retired())?;
            Ok((
                Box::new(GenerationTcp::new(socket, self.fence.clone())) as Box<dyn NativeTcp>,
                address,
            ))
        })
    }
}
struct GenerationTcp {
    socket: Box<dyn NativeTcp>,
    fence: Arc<super::generation::Fence>,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
}
impl GenerationTcp {
    fn new(socket: Box<dyn NativeTcp>, fence: Arc<super::generation::Fence>) -> Self {
        let cancel = fence.clone();
        Self {
            socket,
            fence,
            cancelled: Box::pin(async move { cancel.cancelled().await }),
        }
    }
    fn poll<T>(
        &mut self,
        cx: &mut Context<'_>,
        operation: impl FnOnce(&mut dyn NativeTcp, &mut Context<'_>) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(retired()));
        }
        let Ok(_guard) = self.fence.enter() else {
            return Poll::Ready(Err(retired()));
        };
        operation(self.socket.as_mut(), cx)
    }
}
impl NativeTcp for GenerationTcp {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.socket.peer_addr()
    }
    fn set_nodelay(&self, value: bool) -> io::Result<()> {
        self.socket.set_nodelay(value)
    }
}
impl tokio::io::AsyncRead for GenerationTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.get_mut()
            .poll(cx, |s, cx| Pin::new(s).poll_read(cx, bytes))
    }
}
impl tokio::io::AsyncWrite for GenerationTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut()
            .poll(cx, |s, cx| Pin::new(s).poll_write(cx, bytes))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().poll(cx, |s, cx| Pin::new(s).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut()
            .poll(cx, |s, cx| Pin::new(s).poll_shutdown(cx))
    }
}
