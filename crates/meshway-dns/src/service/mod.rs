//! Bounded loopback-only DNS serving. Installing host DNS is a separate owner.
use crate::{
    Answer, Dispatcher, Query, ResponseTransport, TcpCodec, decode_query, encode_response,
};
use bytes::{Bytes, BytesMut};
use hickory_proto::op::ResponseCode;
use meshway_configuration::DnsTransport;
use std::{
    future::poll_fn,
    io,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWrite},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::Semaphore,
    task::JoinSet,
};
use tokio_util::{
    codec::{Decoder, Encoder},
    sync::CancellationToken,
};

#[derive(Clone, Copy, Debug)]
pub struct ServiceLimits {
    pub max_queries: usize,
    pub max_tcp_connections: usize,
    pub query_timeout: Duration,
    pub tcp_idle_timeout: Duration,
    pub max_tcp_frame_size: u16,
    pub udp_payload_limit: u16,
}

impl Default for ServiceLimits {
    fn default() -> Self {
        Self {
            max_queries: 128,
            max_tcp_connections: 32,
            query_timeout: Duration::from_secs(5),
            tcp_idle_timeout: Duration::from_secs(30),
            max_tcp_frame_size: u16::MAX,
            udp_payload_limit: 1232,
        }
    }
}

impl ServiceLimits {
    fn validate(self) -> io::Result<Self> {
        if self.max_queries == 0
            || self.max_queries > 4096
            || self.max_tcp_connections == 0
            || self.max_tcp_connections > 256
            || self.query_timeout.is_zero()
            || self.query_timeout > Duration::from_secs(60)
            || self.tcp_idle_timeout.is_zero()
            || self.tcp_idle_timeout > Duration::from_secs(300)
            || self.max_tcp_frame_size < 512
            || self.udp_payload_limit < 512
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid DNS service limits",
            ));
        }
        Ok(self)
    }
}

pub struct Service {
    dispatcher: Arc<dyn Dispatcher>,
    limits: ServiceLimits,
}

impl Service {
    pub fn new(dispatcher: Arc<dyn Dispatcher>, limits: ServiceLimits) -> io::Result<Self> {
        Ok(Self {
            dispatcher,
            limits: limits.validate()?,
        })
    }

    /// Consume already-bound matching UDP/TCP loopback listeners. Every peer is
    /// also checked. One sequential query per TCP connection provides bounded
    /// pipelining/backpressure. Excess UDP work and excess connections are dropped.
    /// Shutdown cancels and joins all service tasks before stopping the dispatcher.
    pub async fn run(
        self,
        udp: UdpSocket,
        tcp: TcpListener,
        shutdown: CancellationToken,
    ) -> io::Result<()> {
        validate_binding(udp.local_addr()?, tcp.local_addr()?)?;
        self.serve(udp, tcp, shutdown, Ingress::Loopback).await
    }

    /// Serve Mesh sources on this Node's own Mesh resolver address (port 53),
    /// for a resolver Node's served plan. Every query is handed to
    /// `Dispatcher::dispatch_from` with its peer; the dispatcher alone decides
    /// whether that source may be served.
    pub async fn run_mesh(
        self,
        udp: UdpSocket,
        tcp: TcpListener,
        shutdown: CancellationToken,
    ) -> io::Result<()> {
        let local = udp.local_addr()?;
        if local.ip().is_loopback()
            || local.ip().is_unspecified()
            || local.port() != 53
            || tcp.local_addr()? != local
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Mesh DNS requires matching UDP/TCP bindings on one Mesh address, port 53",
            ));
        }
        self.serve(udp, tcp, shutdown, Ingress::Mesh(local)).await
    }

    async fn serve(
        self,
        udp: UdpSocket,
        tcp: TcpListener,
        shutdown: CancellationToken,
        ingress: Ingress,
    ) -> io::Result<()> {
        let udp = Arc::new(udp);
        let queries = Arc::new(Semaphore::new(self.limits.max_queries));
        let connections = Arc::new(Semaphore::new(self.limits.max_tcp_connections));
        let stop = shutdown.child_token();
        let mut tasks = JoinSet::new();
        // The extra byte detects an oversized UDP DNS message before decoding.
        let mut buffer = vec![0u8; usize::from(u16::MAX) + 1];
        let result = loop {
            tokio::select! { biased;
                _ = stop.cancelled() => break Ok(()),
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    if joined.is_some_and(|result| result.is_err()) {
                        break Err(io::Error::other("DNS service worker failed"));
                    }
                },
                accepted = tcp.accept() => {
                    let (stream, peer) = match accepted { Ok(value) => value, Err(error) => break Err(error) };
                    if !ingress.accepts(peer) { continue; }
                    let Ok(connection) = connections.clone().try_acquire_owned() else { continue; };
                    let dispatcher = self.dispatcher.clone();
                    let queries = queries.clone();
                    let stop = stop.clone();
                    let limits = self.limits;
                    tasks.spawn(async move {
                        let _connection = connection;
                        let _ = serve_tcp(stream, peer, ingress, dispatcher, queries, limits, stop).await;
                    });
                },
                received = udp.recv_from(&mut buffer) => {
                    let (length, peer) = match received { Ok(value) => value, Err(error) => break Err(error) };
                    if !ingress.accepts(peer) { continue; }
                    let Ok(permit) = queries.clone().try_acquire_owned() else { continue; };
                    let Ok(query) = decode_query(&buffer[..length]) else { continue; };
                    let dispatcher = self.dispatcher.clone();
                    let udp = udp.clone();
                    let stop = stop.clone();
                    let limits = self.limits;
                    let deadline = Instant::now() + limits.query_timeout;
                    tasks.spawn(async move {
                        let _permit = permit;
                        let reply = dispatch(dispatcher.as_ref(), (ingress, peer, DnsTransport::Udp), query, deadline, stop.clone()).await;
                        let _ = send_udp(&udp, peer, reply, limits.udp_payload_limit).await;
                    });
                },
            }
        };
        stop.cancel();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        self.dispatcher.stop().await;
        result
    }
}

fn validate_binding(udp: SocketAddr, tcp: SocketAddr) -> io::Result<()> {
    if !udp.ip().is_loopback() || !tcp.ip().is_loopback() || udp != tcp {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "DNS requires matching loopback UDP/TCP bindings",
        ));
    }
    Ok(())
}

/// Empty service failure has no execution authority: it uses only the original
/// client request lifetime and this socket owner's stop token. Expired positive
/// answers are never repurposed as negative results.
struct Reply {
    query: Query,
    answer: Option<Answer>,
    deadline: Instant,
    stop: CancellationToken,
}

impl Reply {
    fn ensure_current(&self) -> io::Result<()> {
        if self.stop.is_cancelled() || Instant::now() >= self.deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "DNS request ended"));
        }
        if let Some(answer) = &self.answer {
            answer
                .ensure_current()
                .map_err(|_| io::Error::other("DNS answer invalidated"))?;
        }
        Ok(())
    }

    fn encode(&self, transport: ResponseTransport, limit: u16) -> io::Result<Bytes> {
        self.ensure_current()?;
        match &self.answer {
            Some(answer) => answer.encode(transport, limit).map_err(io::Error::other),
            None => {
                let mut message = self.query.response();
                message.metadata.response_code = ResponseCode::ServFail;
                encode_response(&self.query, &message, transport, limit).map_err(io::Error::other)
            }
        }
    }

    async fn ended(&self) {
        tokio::select! { biased;
            _ = self.stop.cancelled() => {},
            _ = tokio::time::sleep_until(self.deadline.into()) => {},
            _ = async {
                match &self.answer {
                    Some(answer) => answer.ended().await,
                    None => std::future::pending().await,
                }
            } => {},
        }
    }
}

/// Where queries come from: loopback clients of this Node, or Mesh sources of
/// a served resolver plan arriving at `local`.
#[derive(Clone, Copy)]
enum Ingress {
    Loopback,
    Mesh(SocketAddr),
}

impl Ingress {
    fn accepts(self, peer: SocketAddr) -> bool {
        match self {
            Self::Loopback => peer.ip().is_loopback(),
            Self::Mesh(_) => !peer.ip().is_loopback(),
        }
    }
}

async fn dispatch(
    dispatcher: &dyn Dispatcher,
    (ingress, peer, transport): (Ingress, SocketAddr, DnsTransport),
    query: Query,
    deadline: Instant,
    stop: CancellationToken,
) -> Reply {
    // Reserve bounded time inside the original request deadline to send an
    // empty SERVFAIL on timeout; never extend that deadline for error delivery.
    let remaining = deadline.saturating_duration_since(Instant::now());
    let work_deadline = deadline - (remaining / 10).min(Duration::from_millis(100));
    let answer = tokio::select! { biased;
        _ = stop.cancelled() => None,
        _ = tokio::time::sleep_until(work_deadline.into()) => None,
        result = async { match ingress {
            Ingress::Loopback => dispatcher.dispatch(query.clone(), work_deadline).await,
            Ingress::Mesh(local) => dispatcher.dispatch_from(peer, local, transport, query.clone(), work_deadline).await,
        } } => result.ok().filter(|a| a.answers(&query) && a.ensure_current().is_ok()),
    };
    Reply {
        query,
        answer,
        deadline,
        stop,
    }
}

async fn send_udp(
    socket: &UdpSocket,
    peer: SocketAddr,
    reply: Reply,
    limit: u16,
) -> io::Result<()> {
    let bytes = reply.encode(ResponseTransport::Udp, limit)?;
    let sent = tokio::select! { biased;
        _ = reply.ended() => return Err(io::Error::other("DNS reply invalidated")),
        sent = poll_fn(|cx| {
            reply.ensure_current()?;
            socket.poll_send_to(cx, &bytes, peer)
        }) => sent?,
    };
    if sent != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short DNS datagram",
        ));
    }
    Ok(())
}

async fn serve_tcp(
    mut stream: TcpStream,
    peer: SocketAddr,
    ingress: Ingress,
    dispatcher: Arc<dyn Dispatcher>,
    queries: Arc<Semaphore>,
    limits: ServiceLimits,
    stop: CancellationToken,
) -> io::Result<()> {
    let mut codec = TcpCodec::new(limits.max_tcp_frame_size).map_err(io::Error::other)?;
    let mut buffered = BytesMut::new();
    let cap = usize::from(limits.max_tcp_frame_size) + 2;
    loop {
        let query = tokio::select! { biased;
            _ = stop.cancelled() => return Ok(()),
            result = tokio::time::timeout(limits.tcp_idle_timeout, read_query(&mut stream, &mut codec, &mut buffered, cap)) =>
                result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS TCP idle timeout"))??,
        };
        let Some(query) = query else {
            return Ok(());
        };
        let deadline = Instant::now() + limits.query_timeout;
        let permit = tokio::select! { biased;
            _ = stop.cancelled() => return Ok(()),
            permit = tokio::time::timeout_at(deadline.into(), queries.clone().acquire_owned()) =>
                permit.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS capacity timeout"))?
                    .map_err(io::Error::other)?,
        };
        let reply = dispatch(
            dispatcher.as_ref(),
            (ingress, peer, DnsTransport::Tcp),
            query,
            deadline,
            stop.clone(),
        )
        .await;
        let payload = reply.encode(ResponseTransport::Tcp, limits.udp_payload_limit)?;
        let mut framed = BytesMut::new();
        codec
            .encode(payload, &mut framed)
            .map_err(io::Error::other)?;
        let mut written = 0;
        while written < framed.len() {
            let count = tokio::select! { biased;
                _ = reply.ended() => return Err(io::Error::other("DNS TCP reply invalidated")),
                count = poll_fn(|cx| {
                    reply.ensure_current()?;
                    Pin::new(&mut stream).poll_write(cx, &framed[written..])
                }) => count?,
            };
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "short DNS TCP write",
                ));
            }
            written += count;
        }
        drop(permit);
    }
}

async fn read_query(
    stream: &mut TcpStream,
    codec: &mut TcpCodec,
    buffered: &mut BytesMut,
    cap: usize,
) -> io::Result<Option<Query>> {
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(query) = codec.decode(buffered).map_err(io::Error::other)? {
            return Ok(Some(query));
        }
        let available = cap.saturating_sub(buffered.len()).min(chunk.len());
        if available == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "DNS frame budget exceeded",
            ));
        }
        let count = stream.read(&mut chunk[..available]).await?;
        if count == 0 {
            return codec.decode_eof(buffered).map_err(io::Error::other);
        }
        buffered.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(test)]
mod tests;
