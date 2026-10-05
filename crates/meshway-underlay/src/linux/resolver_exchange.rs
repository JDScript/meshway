//! Business DNS uses an exact sealed local route and the already installed
//! kernel scope. It never calls the bootstrap resolver or exposes a raw socket.
use super::{
    datagram::ProtectedDatagram,
    sockets::{ProtectedTcp, SocketFactory, connect_tcp},
};
use meshway_configuration::DnsTransport;
use meshway_dns::{
    Bytes, BytesMut, DnsFuture, GatewayError, LocalResolverRoute, PayloadCodec, Query,
    ResolverExchange, ResponseTransport, TrustedUpstreamResponse,
};
use meshway_transport::raw::Datagram;
use std::{
    future::{Future, poll_fn},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::codec::{Decoder, Encoder};

impl ResolverExchange for SocketFactory {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        Box::pin(async move {
            tokio::select! { biased;
                _ = route.invalidated() => Err(GatewayError),
                result = exchange(self, route, query) => result,
            }
        })
    }
}
fn current(route: &LocalResolverRoute) -> io::Result<()> {
    route.ensure_current().map_err(io::Error::other)
}
async fn exchange(
    factory: &SocketFactory,
    route: &LocalResolverRoute,
    bytes: Bytes,
) -> Result<Bytes, GatewayError> {
    route.ensure_current().map_err(|_| GatewayError)?;
    let scope = factory
        .shared
        .data(route.snapshot())
        .map_err(|_| GatewayError)?;
    let input = meshway_dns::decode_query(&bytes).map_err(|_| GatewayError)?;
    let mut outgoing = bytes.to_vec();
    let mut id = [0u8; 2];
    getrandom::fill(&mut id).map_err(|_| GatewayError)?;
    outgoing[..2].copy_from_slice(&id);
    let query = meshway_dns::decode_query(&outgoing).map_err(|_| GatewayError)?;
    let endpoint = route.endpoint();
    let reply = match route.transport() {
        DnsTransport::Udp => {
            let local = SocketAddr::new(
                match endpoint {
                    SocketAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                    SocketAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                },
                0,
            );
            route.ensure_current().map_err(|_| GatewayError)?;
            let socket =
                ProtectedDatagram::bind(scope, local, &Default::default(), Some(vec![endpoint]))
                    .map_err(|_| GatewayError)?;
            let sent = poll_fn(|cx| {
                current(route)?;
                socket.poll_send_to(cx, &outgoing, endpoint)
            })
            .await
            .map_err(|_| GatewayError)?;
            if sent != outgoing.len() {
                return Err(GatewayError);
            }
            let limit = query.edns().map_or(512, |e| e.max_payload().max(512));
            let mut buffer = vec![0u8; usize::from(limit) + 1];
            let mut answer = None;
            // Source spoofing or malformed replies cannot cause unbounded work.
            for _ in 0..16 {
                let read = poll_fn(|cx| {
                    current(route)?;
                    socket.poll_recv_from(cx, &mut buffer)
                })
                .await
                .map_err(|_| GatewayError)?;
                if read.source != endpoint || read.truncated || read.copied_len > usize::from(limit)
                {
                    continue;
                }
                let data = &buffer[..read.copied_len];
                if TrustedUpstreamResponse::decode(&query, data, ResponseTransport::Udp, limit)
                    .is_ok()
                {
                    answer = Some(data.to_vec());
                    break;
                }
                tokio::task::yield_now().await;
            }
            answer.ok_or(GatewayError)?
        }
        DnsTransport::Tcp => {
            let timeout = route
                .deadline()
                .saturating_duration_since(std::time::Instant::now())
                .min(factory.timeout);
            let options = Default::default();
            let tcp = Default::default();
            let mut connection =
                Box::pin(connect_tcp(scope, endpoint, None, &options, &tcp, timeout));
            let mut stream = poll_fn(|cx| {
                if route.ensure_current().is_err() {
                    return std::task::Poll::Ready(Err(crate::Error::Expired));
                }
                connection.as_mut().poll(cx)
            })
            .await
            .map_err(|_| GatewayError)?;
            if stream.peer_addr().map_err(|_| GatewayError)? != endpoint {
                return Err(GatewayError);
            }
            let mut codec = PayloadCodec::new(u16::MAX).map_err(|_| GatewayError)?;
            let mut frame = BytesMut::new();
            codec
                .encode(Bytes::from(outgoing), &mut frame)
                .map_err(|_| GatewayError)?;
            write(&mut stream, route, &frame)
                .await
                .map_err(|_| GatewayError)?;
            frame.clear();
            let reply = loop {
                if let Some(payload) = codec.decode(&mut frame).map_err(|_| GatewayError)? {
                    break payload;
                }
                let remaining = (usize::from(u16::MAX) + 2).saturating_sub(frame.len());
                if remaining == 0 {
                    return Err(GatewayError);
                }
                let mut buffer = [0u8; 4096];
                let count = read(&mut stream, route, &mut buffer[..remaining.min(4096)])
                    .await
                    .map_err(|_| GatewayError)?;
                if count == 0 {
                    return Err(GatewayError);
                }
                frame.extend_from_slice(&buffer[..count]);
            };
            validate(&query, &reply, ResponseTransport::Tcp)?;
            reply.to_vec()
        }
    };
    route.ensure_current().map_err(|_| GatewayError)?;
    let mut reply = reply;
    reply[..2].copy_from_slice(&input.id().to_be_bytes());
    Ok(Bytes::from(reply))
}
fn validate(query: &Query, bytes: &[u8], transport: ResponseTransport) -> Result<(), GatewayError> {
    TrustedUpstreamResponse::decode(query, bytes, transport, u16::MAX)
        .map(|_| ())
        .map_err(|_| GatewayError)
}
async fn write(
    stream: &mut ProtectedTcp,
    route: &LocalResolverRoute,
    bytes: &[u8],
) -> io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let count = poll_fn(|cx| {
            current(route)?;
            Pin::new(&mut *stream).poll_write(cx, &bytes[written..])
        })
        .await?;
        if count == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        written += count;
    }
    Ok(())
}
async fn read(
    stream: &mut ProtectedTcp,
    route: &LocalResolverRoute,
    bytes: &mut [u8],
) -> io::Result<usize> {
    poll_fn(|cx| {
        current(route)?;
        let mut buffer = ReadBuf::new(bytes);
        Pin::new(&mut *stream)
            .poll_read(cx, &mut buffer)
            .map_ok(|()| buffer.filled().len())
    })
    .await
}
