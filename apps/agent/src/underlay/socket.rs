use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::sync::WaitForCancellationFutureOwned;

use super::{Purpose, closed, generation::Generation};

#[derive(Clone)]
pub(crate) struct SocketFactory(pub(super) Arc<Generation>);

impl SocketFactory {
    pub(crate) fn generation(&self) -> u64 {
        self.0.reservation.generation
    }

    pub(super) fn socket(
        &self,
        purpose: Purpose,
        address: SocketAddr,
        kind: Type,
        protocol: Protocol,
    ) -> io::Result<Arc<Socket>> {
        let state = self.0.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        let socket = Socket::new(Domain::for_address(address), kind, Some(protocol))?;
        socket.set_nonblocking(true)?;
        #[cfg(target_os = "linux")]
        if self.0.reservation.foreign_bits != 0 {
            socket.set_mark(socket.mark()? | self.0.reservation.foreign_bits)?;
        }
        // Before bind, connect, listen, or handing an FD to any async executor.
        self.0.reservation.mark.protect(&socket)?;
        drop(state);
        self.0.register(Arc::new(socket), purpose)
    }

    pub(crate) async fn tcp(
        &self,
        purpose: Purpose,
        remote: SocketAddr,
    ) -> io::Result<ProtectedTcp> {
        self.tcp_bound(purpose, remote, None).await
    }

    pub(crate) async fn tcp_bound(
        &self,
        purpose: Purpose,
        remote: SocketAddr,
        local: Option<SocketAddr>,
    ) -> io::Result<ProtectedTcp> {
        let socket = self.socket(purpose, remote, Type::STREAM, Protocol::TCP)?;
        {
            let state = self.0.state.lock().map_err(|_| closed())?;
            if !state.active {
                return Err(closed());
            }
            self.0.reservation.mark.verify(&socket)?;
            if let Some(local) = local {
                if local.is_ipv4() != remote.is_ipv4() {
                    return Err(super::invalid("TCP local/remote family mismatch"));
                }
                socket.bind(&local.into())?;
            }
            if let Err(error) = socket.connect(&remote.into()) {
                // EINPROGRESS; no other error permits an alternative unmarked path.
                if error.raw_os_error() != Some(115) {
                    return Err(error);
                }
            }
        }
        let stream = TcpStream::from_std(socket.try_clone()?.into())?;
        tokio::select! {
            biased;
            _ = self.0.cancelled.cancelled() => return Err(closed()),
            ready = stream.writable() => ready?,
        }
        if let Some(error) = stream.take_error()? {
            return Err(error);
        }
        // connect success must not publish a retired FD.
        let state = self.0.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        self.0.reservation.mark.verify(&socket)?;
        Ok(ProtectedTcp::new(stream, socket, self.0.clone(), purpose))
    }

    pub(crate) fn udp(&self, purpose: Purpose, bind: SocketAddr) -> io::Result<ProtectedUdp> {
        self.udp_buffered(purpose, bind, 0)
    }

    pub(crate) fn udp_buffered(
        &self,
        purpose: Purpose,
        bind: SocketAddr,
        receive_buffer_bytes: usize,
    ) -> io::Result<ProtectedUdp> {
        let socket = self.socket(purpose, bind, Type::DGRAM, Protocol::UDP)?;
        let state = self.0.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        self.0.reservation.mark.verify(&socket)?;
        if receive_buffer_bytes != 0 {
            socket.set_recv_buffer_size(receive_buffer_bytes)?;
        }
        socket.bind(&bind.into())?;
        let udp = UdpSocket::from_std(socket.try_clone()?.into())?;
        Ok(ProtectedUdp {
            udp,
            socket,
            generation: self.0.clone(),
            purpose,
        })
    }

    pub(crate) fn listener(
        &self,
        purpose: Purpose,
        bind: SocketAddr,
    ) -> io::Result<ProtectedListener> {
        let socket = self.socket(purpose, bind, Type::STREAM, Protocol::TCP)?;
        let state = self.0.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        self.0.reservation.mark.verify(&socket)?;
        // Rebind after generation retirement without sharing live listeners.
        socket.set_reuse_address(true)?;
        socket.bind(&bind.into())?;
        socket.listen(128)?;
        let listener = TcpListener::from_std(socket.try_clone()?.into())?;
        Ok(ProtectedListener {
            listener,
            _socket: socket,
            generation: self.0.clone(),
            purpose,
        })
    }
}

pub(crate) struct ProtectedUdp {
    udp: UdpSocket,
    socket: Arc<Socket>,
    generation: Arc<Generation>,
    purpose: Purpose,
}

impl ProtectedUdp {
    pub(crate) async fn writable(&self) -> io::Result<()> {
        tokio::select! {
            biased;
            _ = self.generation.cancelled.cancelled() => Err(closed()),
            result = self.udp.writable() => result,
        }
    }

    pub(crate) fn try_send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize> {
        let state = self.generation.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        self.generation.reservation.mark.verify(&self.socket)?;
        self.udp.try_send_to(bytes, remote)
    }
    pub(crate) fn purpose(&self) -> Purpose {
        self.purpose
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation.reservation.generation
    }
    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.udp.local_addr()
    }

    pub(crate) async fn connect(&self, remote: SocketAddr) -> io::Result<()> {
        // socket2::connect is immediate for UDP and performed under the fence.
        let state = self.generation.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Err(closed());
        }
        self.generation.reservation.mark.verify(&self.socket)?;
        self.socket.connect(&remote.into())
    }

    pub(crate) async fn send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize> {
        loop {
            tokio::select! {
                biased;
                _ = self.generation.cancelled.cancelled() => return Err(closed()),
                ready = self.udp.writable() => ready?,
            }
            let state = self.generation.state.lock().map_err(|_| closed())?;
            if !state.active {
                return Err(closed());
            }
            self.generation.reservation.mark.verify(&self.socket)?;
            match self.udp.try_send_to(bytes, remote) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                result => return result,
            }
        }
    }

    pub(crate) async fn send(&self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            tokio::select! {
                biased;
                _ = self.generation.cancelled.cancelled() => return Err(closed()),
                ready = self.udp.writable() => ready?,
            }
            let state = self.generation.state.lock().map_err(|_| closed())?;
            if !state.active {
                return Err(closed());
            }
            self.generation.reservation.mark.verify(&self.socket)?;
            match self.udp.try_send(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                result => return result,
            }
        }
    }

    pub(crate) async fn recv_from(&self, bytes: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        loop {
            tokio::select! {
                biased;
                _ = self.generation.cancelled.cancelled() => return Err(closed()),
                ready = self.udp.readable() => ready?,
            }
            let state = self.generation.state.lock().map_err(|_| closed())?;
            if !state.active {
                return Err(closed());
            }
            match self.udp.try_recv_from(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                result => return result,
            }
        }
    }
}

pub(crate) struct ProtectedListener {
    listener: TcpListener,
    _socket: Arc<Socket>,
    generation: Arc<Generation>,
    purpose: Purpose,
}

impl ProtectedListener {
    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub(crate) async fn accept(&self) -> io::Result<(ProtectedTcp, SocketAddr)> {
        let (stream, address) = tokio::select! {
            biased;
            _ = self.generation.cancelled.cancelled() => return Err(closed()),
            result = self.listener.accept() => result?,
        };
        let standard = stream.into_std()?;
        let socket = Socket::from(standard);
        // Verify inherited mark, never repair after SYN-ACK has already escaped.
        self.generation.reservation.mark.verify(&socket)?;
        #[cfg(target_os = "linux")]
        if socket.mark()? != self._socket.mark()? {
            return Err(io::Error::other(
                "accepted socket did not inherit complete listener mark",
            ));
        }
        let socket = self.generation.register(Arc::new(socket), self.purpose)?;
        let stream = TcpStream::from_std(socket.try_clone()?.into())?;
        Ok((
            ProtectedTcp::new(stream, socket, self.generation.clone(), self.purpose),
            address,
        ))
    }
}

pub(crate) struct ProtectedTcp {
    stream: TcpStream,
    socket: Arc<Socket>,
    generation: Arc<Generation>,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
    purpose: Purpose,
}

impl ProtectedTcp {
    fn new(
        stream: TcpStream,
        socket: Arc<Socket>,
        generation: Arc<Generation>,
        purpose: Purpose,
    ) -> Self {
        let cancelled = Box::pin(generation.cancelled.clone().cancelled_owned());
        Self {
            stream,
            socket,
            generation,
            cancelled,
            purpose,
        }
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }
    pub(crate) fn purpose(&self) -> Purpose {
        self.purpose
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation.reservation.generation
    }

    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }
    pub(crate) fn set_nodelay(&self, value: bool) -> io::Result<()> {
        self.stream.set_nodelay(value)
    }
}

impl AsyncRead for ProtectedTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed()));
        }
        let state = this.generation.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut this.stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for ProtectedTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed()));
        }
        let state = this.generation.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Poll::Ready(Err(closed()));
        }
        this.generation.reservation.mark.verify(&this.socket)?;
        Pin::new(&mut this.stream).poll_write(cx, bytes)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.cancelled.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(closed()));
        }
        let state = this.generation.state.lock().map_err(|_| closed())?;
        if !state.active {
            return Poll::Ready(Err(closed()));
        }
        Pin::new(&mut this.stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

impl Drop for ProtectedTcp {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
    }
}
