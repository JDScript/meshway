//! One sealed-route DNS exchange implementation for all physical hosts.
use crate::{
    Bytes, BytesMut, DnsFuture, GatewayError, LocalResolverRoute, PayloadCodec, ResponseTransport,
    TrustedUpstreamResponse,
};
use meshway_configuration::DnsTransport;
use meshway_transport::raw::{Datagram, Stream};
use std::{future::poll_fn, io, net::SocketAddr, pin::Pin};
use tokio::io::ReadBuf;
use tokio_util::codec::{Decoder, Encoder};

pub struct Connection {
    pub stream: Box<dyn Stream>,
    pub peer: SocketAddr,
}
/// Open the sealed route's endpoint, with the platform's socket protection.
/// Wire validation and every subsequent I/O poll are owned by `exchange`.
pub trait Sockets: Send + Sync {
    fn udp<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
    ) -> DnsFuture<'a, io::Result<Box<dyn Datagram>>>;
    fn tcp<'a>(&'a self, route: &'a LocalResolverRoute) -> DnsFuture<'a, io::Result<Connection>>;
}
pub fn exchange<'a>(
    factory: &'a dyn Sockets,
    route: &'a LocalResolverRoute,
    query: Bytes,
) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
    Box::pin(async move {
        tokio::select! { biased;
            _ = route.ended() => Err(GatewayError),
            result = perform(factory, route, query) => result,
        }
    })
}
fn current(route: &LocalResolverRoute) -> io::Result<()> {
    route.ensure_current().map_err(io::Error::other)
}
async fn perform(
    factory: &dyn Sockets,
    route: &LocalResolverRoute,
    bytes: Bytes,
) -> Result<Bytes, GatewayError> {
    route.ensure_current().map_err(|_| GatewayError)?;
    let input = crate::decode_query(&bytes).map_err(|_| GatewayError)?;
    let mut outgoing = bytes.to_vec();
    let mut id = [0; 2];
    getrandom::fill(&mut id).map_err(|_| GatewayError)?;
    outgoing[..2].copy_from_slice(&id);
    let query = crate::decode_query(&outgoing).map_err(|_| GatewayError)?;
    let endpoint = route.endpoint();
    let mut reply = match route.transport() {
        DnsTransport::Udp => {
            route.ensure_current().map_err(|_| GatewayError)?;
            let socket = factory.udp(route).await.map_err(|_| GatewayError)?;
            let sent = poll_fn(|cx| {
                current(route)?;
                socket.poll_send_to(cx, &outgoing, endpoint)
            })
            .await
            .map_err(|_| GatewayError)?;
            if sent != outgoing.len() {
                return Err(GatewayError);
            }
            let limit = query.edns().map_or(512, |edns| edns.max_payload().max(512));
            let mut buffer = vec![0; usize::from(limit) + 1];
            let mut answer = None;
            for _ in 0..16 {
                let read = poll_fn(|cx| {
                    current(route)?;
                    socket.poll_recv_from(cx, &mut buffer)
                })
                .await
                .map_err(|_| GatewayError)?;
                if read.source != endpoint {
                    continue;
                }
                let data = &buffer[..read.copied_len.min(usize::from(limit))];
                let oversized = read.truncated || read.copied_len > usize::from(limit);
                if !oversized
                    && TrustedUpstreamResponse::decode(&query, data, ResponseTransport::Udp, limit)
                        .is_ok()
                {
                    answer = Some(data.to_vec());
                    break;
                }
                if data.len() >= 12
                    && data[..2] == outgoing[..2]
                    && data[2] & 0x80 != 0
                    && (oversized || data[2] & 0x02 != 0)
                {
                    let mut truncated = query.response();
                    truncated.metadata.truncation = true;
                    answer = Some(
                        crate::encode_response(&query, &truncated, ResponseTransport::Udp, limit)
                            .map_err(|_| GatewayError)?
                            .to_vec(),
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
            answer.ok_or(GatewayError)?
        }
        DnsTransport::Tcp => {
            let mut connecting = factory.tcp(route);
            let connection = poll_fn(|cx| {
                current(route)?;
                connecting.as_mut().poll(cx)
            })
            .await
            .map_err(|_| GatewayError)?;
            if connection.peer != endpoint {
                return Err(GatewayError);
            }
            let mut stream = connection.stream;
            let mut codec = PayloadCodec::new(u16::MAX).map_err(|_| GatewayError)?;
            let mut frame = BytesMut::new();
            codec
                .encode(Bytes::from(outgoing), &mut frame)
                .map_err(|_| GatewayError)?;
            let mut written = 0;
            while written < frame.len() {
                let count = poll_fn(|cx| {
                    current(route)?;
                    Pin::new(&mut *stream).poll_write(cx, &frame[written..])
                })
                .await
                .map_err(|_| GatewayError)?;
                if count == 0 {
                    return Err(GatewayError);
                }
                written += count;
            }
            frame.clear();
            let reply = loop {
                if let Some(payload) = codec.decode(&mut frame).map_err(|_| GatewayError)? {
                    break payload;
                }
                let remaining = (usize::from(u16::MAX) + 2).saturating_sub(frame.len());
                if remaining == 0 {
                    return Err(GatewayError);
                }
                let mut bytes = [0; 4096];
                let count = poll_fn(|cx| {
                    current(route)?;
                    let mut buffer = ReadBuf::new(&mut bytes[..remaining.min(4096)]);
                    Pin::new(&mut *stream)
                        .poll_read(cx, &mut buffer)
                        .map_ok(|()| buffer.filled().len())
                })
                .await
                .map_err(|_| GatewayError)?;
                if count == 0 {
                    return Err(GatewayError);
                }
                frame.extend_from_slice(&bytes[..count]);
            };
            TrustedUpstreamResponse::decode(&query, &reply, ResponseTransport::Tcp, u16::MAX)
                .map_err(|_| GatewayError)?;
            reply.to_vec()
        }
    };
    route.ensure_current().map_err(|_| GatewayError)?;
    reply[..2].copy_from_slice(&input.id().to_be_bytes());
    Ok(Bytes::from(reply))
}
