use super::super::resources::Owned;
use super::*;
use meshway_transport::spec::{Listener, SocketOptions, TcpOptions};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpSocket, TcpStream},
};

struct Stream(Owned<TcpStream>);
impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.poll(cx, false, |socket, cx| {
            Pin::new(socket).poll_read(cx, buffer)
        })
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.poll(cx, true, |socket, cx| {
            Pin::new(socket).poll_write(cx, bytes)
        })
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0
            .poll(cx, true, |socket, cx| Pin::new(socket).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0
            .poll(cx, true, |socket, cx| Pin::new(socket).poll_shutdown(cx))
    }
}

pub(super) fn adopt(
    stream: TcpStream,
    resource: Arc<Resource>,
    socket: &SocketOptions,
    tcp: &TcpOptions,
) -> io::Result<AcceptedSocket> {
    resource.check()?;
    let stream = meshway_transport::tcp::TcpTransport::adopt(stream, socket, tcp)?.into_inner();
    Ok(AcceptedSocket {
        local: stream.local_addr()?,
        peer: stream.peer_addr()?,
        stream: Box::new(Stream(Owned::new(stream, resource)?)),
    })
}
fn socket(ipv4: bool) -> io::Result<TcpSocket> {
    if ipv4 {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
}

pub(super) async fn connect(
    factory: &Factory,
    desired: &Desired,
    key: &str,
    endpoint: &DialEndpoint,
) -> Result<AcceptedSocket, Error> {
    let resource = factory.resource(desired, key, false)?;
    let (enabled, interface, local, options, tcp, timeout) =
        match &desired.dialers()[key].parameters {
            DialParameters::NativeTcp { options: v } => (
                v.enabled,
                &v.bind_interface,
                &v.local_address,
                &v.socket,
                &v.tcp,
                v.connect_timeout.as_std(),
            ),
            DialParameters::Vless { options: v, .. } => (
                v.enabled,
                &v.bind_interface,
                &v.local_address,
                &v.socket,
                &v.tcp,
                v.connect_timeout.as_std(),
            ),
            DialParameters::Trojan { options: v, .. } => (
                v.enabled,
                &v.bind_interface,
                &v.local_address,
                &v.socket,
                &v.tcp,
                v.connect_timeout.as_std(),
            ),
            _ => return Err(Error::Configuration),
        };
    if !enabled {
        return Err(Error::Configuration);
    }
    let local = if local.is_empty() {
        None
    } else {
        Some(local.parse::<IpAddr>().map_err(|_| Error::Configuration)?)
    };
    let addresses = factory.resolve(&resource, &endpoint.endpoint()).await?;
    let connecting = async {
        let mut last = io::Error::from(io::ErrorKind::NetworkUnreachable);
        for address in addresses {
            if local.is_some_and(|local| local.is_ipv4() != address.is_ipv4()) {
                continue;
            }
            resource.check()?;
            let socket = socket(address.is_ipv4())?;
            factory.protect(
                &socket2::SockRef::from(&socket),
                address.is_ipv4(),
                interface,
            )?;
            meshway_transport::raw::apply_socket_options(
                &socket2::SockRef::from(&socket),
                options,
            )?;
            if let Some(local) = local {
                socket.bind(SocketAddr::new(local, 0))?;
            }
            match socket.connect(address).await {
                Ok(stream) => return adopt(stream, resource.clone(), options, tcp),
                Err(error) => last = error,
            }
        }
        Err(last)
    };
    tokio::select! { biased;
        _ = resource.stop.cancelled() => Err(Error::Underlay),
        result = tokio::time::timeout(timeout.min(factory.timeout), connecting) => Ok(result.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??),
    }
}

struct Listening {
    socket: Owned<TcpListener>,
    factory: Factory,
    options: SocketOptions,
    tcp: TcpOptions,
}
impl ListenerSocket for Listening {
    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.socket.with(|socket| socket.local_addr())?)
    }
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            let (stream, _) = std::future::poll_fn(|cx| {
                self.socket
                    .poll(cx, false, |socket, cx| socket.poll_accept(cx))
            })
            .await?;
            let desired = self.factory.resources.execution(&self.socket.resource)?;
            let resource = self.factory.resources.acquire(
                &desired,
                self.socket
                    .resource
                    .purpose
                    .clone()
                    .ok_or(Error::Underlay)?,
            )?;
            Ok(adopt(stream, resource, &self.options, &self.tcp)?)
        })
    }
}
pub(super) fn listen(
    factory: &Factory,
    desired: &Desired,
    key: &str,
) -> Result<Box<dyn ListenerSocket>, Error> {
    let resource = factory.resource(desired, key, true)?;
    let (enabled, interface, address, options, tcp) =
        match desired.local().listeners[key].declaration() {
            Listener::NativeTcp(v) => (
                v.enabled,
                &v.bind_interface,
                SocketAddr::new(v.listen, v.listen_port.get()),
                &v.socket,
                &v.tcp,
            ),
            Listener::Vless(v) => (
                v.enabled,
                &v.bind_interface,
                SocketAddr::new(v.listen, v.listen_port.get()),
                &v.socket,
                &v.tcp,
            ),
            Listener::Trojan(v) => (
                v.enabled,
                &v.bind_interface,
                SocketAddr::new(v.listen, v.listen_port.get()),
                &v.socket,
                &v.tcp,
            ),
            _ => return Err(Error::Configuration),
        };
    if !enabled {
        return Err(Error::Configuration);
    }
    let socket = socket(address.is_ipv4())?;
    factory.protect(
        &socket2::SockRef::from(&socket),
        address.is_ipv4(),
        interface,
    )?;
    socket.set_reuseaddr(true)?;
    meshway_transport::raw::apply_socket_options(&socket2::SockRef::from(&socket), options)?;
    socket.bind(address).map_err(|source| Error::Bind {
        key: key.into(),
        address,
        source,
    })?;
    let socket = Owned::new(socket.listen(128)?, resource)?;
    Ok(Box::new(Listening {
        socket,
        factory: factory.clone(),
        options: options.clone(),
        tcp: tcp.clone(),
    }))
}
