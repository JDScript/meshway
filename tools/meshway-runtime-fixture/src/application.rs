//! Ordinary kernel sockets only. No Meshway control, packet codec or crypto here.
use crate::{Error, IO, Result, STARTUP, signals::Addresses};
use serde::Serialize;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const UDP: u16 = 42424;
const TCP: u16 = 42425;
pub const SUBSCRIPTION_PORT: u16 = 42428;
const FRAME: usize = 64;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub udp4: u64,
    pub udp6: u64,
    pub tcp4: u64,
    pub tcp6: u64,
}

pub struct Echo {
    stop: CancellationToken,
    tasks: JoinSet<Result<()>>,
    counts: Arc<[AtomicU64; 4]>,
}

impl Echo {
    pub async fn start(addresses: &Addresses, underlay: std::net::Ipv4Addr) -> Result<Self> {
        // Bind all resources before spawning, so partial startup owns no tasks.
        let udp4 = UdpSocket::bind((addresses.ipv4, UDP)).await?;
        let udp6 = UdpSocket::bind((addresses.ipv6, UDP)).await?;
        let tcp4 = TcpListener::bind((addresses.ipv4, TCP)).await?;
        let tcp6 = TcpListener::bind((addresses.ipv6, TCP)).await?;
        let local = TcpListener::bind((underlay, 42426)).await?;
        let publisher = UdpSocket::bind((addresses.ipv4, SUBSCRIPTION_PORT)).await?;
        let stop = CancellationToken::new();
        let counts = Arc::new(std::array::from_fn(|_| AtomicU64::new(0)));
        let mut tasks = JoinSet::new();
        for (index, socket) in [(0, udp4), (1, udp6)] {
            let stop = stop.clone();
            let counts = counts.clone();
            tasks.spawn(async move {
                let mut buffer = [0; FRAME + 1];
                loop {
                    tokio::select! {
                        () = stop.cancelled() => return Ok(()),
                        packet = socket.recv_from(&mut buffer) => {
                            let (length, from) = packet?;
                            if length != FRAME { return Err("unexpected UDP application payload".into()); }
                            socket.send_to(&buffer[..length], from).await?;
                            counts[index].fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
        for (index, listener) in [(2, tcp4), (3, tcp6)] {
            let stop = stop.clone();
            let counts = counts.clone();
            tasks.spawn(tcp_echo(listener, stop, counts, index));
        }
        tasks.spawn(tcp_echo(
            local,
            stop.clone(),
            Arc::new(std::array::from_fn(|_| AtomicU64::new(0))),
            0,
        ));
        tasks.spawn(publish_subscription(publisher, stop.clone()));
        Ok(Self {
            stop,
            tasks,
            counts,
        })
    }
    pub fn counts(&self) -> Counts {
        let read = |index: usize| self.counts[index].load(Ordering::Relaxed);
        Counts {
            udp4: read(0),
            udp6: read(1),
            tcp4: read(2),
            tcp6: read(3),
        }
    }
    pub fn check(&mut self) -> Result<()> {
        if let Some(result) = self.tasks.try_join_next() {
            result??;
            return Err("echo task stopped before fixture shutdown".into());
        }
        Ok(())
    }
    pub async fn shutdown(&mut self) -> Result<()> {
        self.stop.cancel();
        let mut result = Ok(());
        while let Some(joined) = self.tasks.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => result = Err(error),
                Err(error) => result = Err(error.into()),
            }
        }
        result
    }
}

/// One subscriber, one OS socket, one bounded task owned/joined by Echo. The
/// separate port keeps ordinary request/echo traffic out of this demand.
async fn publish_subscription(socket: UdpSocket, stop: CancellationToken) -> Result<()> {
    let mut subscriber: Option<(SocketAddr, [u8; 16])> = None;
    let mut sequence = 0_u64;
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut bytes = [0; FRAME + 1];
    loop {
        tokio::select! {
            () = stop.cancelled() => return Ok(()),
            received = socket.recv_from(&mut bytes) => {
                let (size, peer) = match received {
                    Ok(value) => value,
                    Err(error) if publisher_blocked(&error) => continue,
                    Err(error) => return Err(error.into()),
                };
                if size != FRAME || bytes[16..FRAME].iter().any(|byte| *byte != 0) {
                    return Err("invalid stream subscription challenge".into());
                }
                let token: [u8; 16] = bytes[..16].try_into()?;
                match subscriber {
                    Some(current) if current != (peer, token) => return Err("stream subscriber bound exceeded".into()),
                    Some(_) => {}, // identical initial retries never restart the publisher
                    None => subscriber = Some((peer, token)),
                }
            }
            _ = interval.tick() => {
                let Some((peer, token)) = subscriber else { continue; };
                sequence = sequence.checked_add(1).ok_or("stream sequence exhausted")?;
                let mut frame = [0; FRAME];
                frame[..16].copy_from_slice(&token);
                frame[16..24].copy_from_slice(&sequence.to_be_bytes());
                tokio::select! {
                    () = stop.cancelled() => return Ok(()),
                    sent = socket.send_to(&frame, peer) => match sent {
                        Ok(FRAME) => {},
                        Ok(_) => return Err("short stream publication".into()),
                        Err(error) if publisher_blocked(&error) => {},
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
    }
}

fn publisher_blocked(error: &std::io::Error) -> bool {
    expected_block(error) || error.kind() == std::io::ErrorKind::AddrNotAvailable
}

/// After start returns there is deliberately no sending API. Every subsequent
/// operation on this original socket only drains/receives publisher messages.
pub struct Subscription {
    socket: UdpSocket,
    evidence: SubscriptionEvidence,
    stopped_sending: Instant,
    born: Instant,
    last_received: Option<Instant>,
    last_received_wall_ms: Option<u64>,
    last_fresh: Option<Instant>,
    last_fresh_wall_ms: Option<u64>,
    observation: Arc<Mutex<Option<SubscriptionFailureEvidence>>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionEvidence {
    pub local_tuple: SocketAddr,
    pub peer_tuple: SocketAddr,
    pub token: [u8; 16],
    pub sent_initial_count: u64,
    pub post_start_sent_count: u64,
    pub received_count: u64,
    pub drained_count: u64,
    pub max_sequence: u64,
    pub start_wall_ms: u64,
    pub end_wall_ms: u64,
    pub receive_only_elapsed_ms: u128,
}

/// Failure-only observation. It is not a successful conformance result.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionFailureEvidence {
    passed: bool,
    local_tuple: SocketAddr,
    peer_tuple: SocketAddr,
    monotonic_origin: &'static str,
    receive_only_start_monotonic_ms: u128,
    receive_only_elapsed_ms: u128,
    observed_wall_ms: Option<u64>,
    last_received_monotonic_ms: Option<u128>,
    last_received_wall_ms: Option<u64>,
    last_fresh_monotonic_ms: Option<u128>,
    last_fresh_wall_ms: Option<u64>,
    max_sequence: Option<u64>,
    received_count: Option<u64>,
    drained_count: Option<u64>,
    sent_initial_count: u64,
    post_start_sent_count: u64,
}

impl Subscription {
    pub async fn start(source: &Addresses, destination: &Addresses) -> Result<Self> {
        let socket = UdpSocket::bind((source.ipv4, 0)).await?;
        socket
            .connect((destination.ipv4, SUBSCRIPTION_PORT))
            .await?;
        let mut token = [0; 16];
        getrandom::fill(&mut token).map_err(|_| "stream challenge entropy unavailable")?;
        let mut frame = [0; FRAME];
        frame[..16].copy_from_slice(&token);
        let born = Instant::now();
        let mut subscription = Self {
            evidence: SubscriptionEvidence {
                local_tuple: socket.local_addr()?,
                peer_tuple: socket.peer_addr()?,
                token,
                sent_initial_count: 0,
                post_start_sent_count: 0,
                received_count: 0,
                drained_count: 0,
                max_sequence: 0,
                start_wall_ms: wall_ms()?,
                end_wall_ms: wall_ms()?,
                receive_only_elapsed_ms: 0,
            },
            socket,
            stopped_sending: Instant::now(),
            born,
            last_received: None,
            last_received_wall_ms: None,
            last_fresh: None,
            last_fresh_wall_ms: None,
            observation: Arc::new(Mutex::new(None)),
        };
        let deadline = Instant::now() + STARTUP;
        loop {
            if subscription.socket.send(&frame).await? != FRAME {
                return Err("short subscription write".into());
            }
            subscription.evidence.sent_initial_count += 1;
            if let Ok(result) =
                tokio::time::timeout(Duration::from_millis(500), subscription.receive_new()).await
            {
                result?;
                subscription.stopped_sending = Instant::now();
                subscription.evidence.start_wall_ms = wall_ms()?;
                subscription.evidence.post_start_sent_count =
                    subscription.evidence.sent_initial_count;
                return Ok(subscription);
            }
            if Instant::now() >= deadline {
                return Err("stream subscription did not produce a reply".into());
            }
        }
    }

    fn observe(&mut self, bytes: &[u8], drained: bool) -> Result<bool> {
        if bytes.len() != FRAME
            || bytes[..16] != self.evidence.token
            || bytes[24..].iter().any(|byte| *byte != 0)
        {
            return Err("stream reply challenge mismatch".into());
        }
        let sequence = u64::from_be_bytes(bytes[16..24].try_into()?);
        if sequence == 0 {
            return Err("invalid stream reply sequence".into());
        }
        self.evidence.received_count += 1;
        self.last_received = Some(Instant::now());
        self.last_received_wall_ms = wall_ms().ok();
        if drained {
            self.evidence.drained_count += 1;
        }
        if sequence <= self.evidence.max_sequence {
            return Ok(false);
        }
        self.evidence.max_sequence = sequence;
        self.last_fresh = self.last_received;
        self.last_fresh_wall_ms = self.last_received_wall_ms;
        Ok(true)
    }

    fn drain(&mut self) -> Result<()> {
        let mut bytes = [0; FRAME + 1];
        for _ in 0..256 {
            match self.socket.try_recv(&mut bytes) {
                Ok(size) => {
                    self.observe(&bytes[..size], true)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error.into()),
            }
        }
        Err("stream receive backlog bound exceeded".into())
    }

    async fn receive_new(&mut self) -> Result<()> {
        let mut bytes = [0; FRAME + 1];
        loop {
            let size = self.socket.recv(&mut bytes).await?;
            if self.observe(&bytes[..size], false)? {
                return Ok(());
            }
        }
    }

    pub async fn receive_only(mut self) -> Result<SubscriptionEvidence> {
        loop {
            // Empty the socket first, then require a newer reply to arrive.
            // In particular the final check starts after 125 receive-only
            // seconds, so an old queued datagram cannot satisfy that boundary.
            self.drain()?;
            let final_check = self.stopped_sending.elapsed() >= Duration::from_secs(125);
            tokio::time::timeout(IO, self.receive_new())
                .await
                .map_err(|_| "receive-only UDP stream stopped")??;
            if self.socket.local_addr()? != self.evidence.local_tuple
                || self.socket.peer_addr()? != self.evidence.peer_tuple
                || self.evidence.post_start_sent_count != self.evidence.sent_initial_count
            {
                return Err("receive-only subscription changed socket or sent again".into());
            }
            if final_check {
                break;
            }
        }
        self.evidence.end_wall_ms = wall_ms()?;
        self.evidence.receive_only_elapsed_ms = self.stopped_sending.elapsed().as_millis();
        Ok(self.evidence.clone())
    }

    pub fn observation(&self) -> Arc<Mutex<Option<SubscriptionFailureEvidence>>> {
        Arc::clone(&self.observation)
    }

    fn failure_evidence(&self) -> SubscriptionFailureEvidence {
        let measured = self.last_received.is_some();
        SubscriptionFailureEvidence {
            passed: false,
            local_tuple: self.evidence.local_tuple,
            peer_tuple: self.evidence.peer_tuple,
            monotonic_origin: "subscription_created",
            receive_only_start_monotonic_ms: self
                .stopped_sending
                .duration_since(self.born)
                .as_millis(),
            receive_only_elapsed_ms: self.stopped_sending.elapsed().as_millis(),
            observed_wall_ms: wall_ms().ok(),
            last_received_monotonic_ms: self
                .last_received
                .map(|at| at.duration_since(self.born).as_millis()),
            last_received_wall_ms: self.last_received_wall_ms,
            last_fresh_monotonic_ms: self
                .last_fresh
                .map(|at| at.duration_since(self.born).as_millis()),
            last_fresh_wall_ms: self.last_fresh_wall_ms,
            max_sequence: self.last_fresh.map(|_| self.evidence.max_sequence),
            received_count: measured.then_some(self.evidence.received_count),
            drained_count: measured.then_some(self.evidence.drained_count),
            sent_initial_count: self.evidence.sent_initial_count,
            post_start_sent_count: self.evidence.post_start_sent_count,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // Capture on success, error, or cancellation, while retaining the exact
        // previous socket-drop boundary. Only the actor's failure branch saves it.
        if let Ok(mut observation) = self.observation.lock() {
            *observation = Some(self.failure_evidence());
        }
    }
}

fn wall_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

pub async fn underlay_probe(address: std::net::Ipv4Addr) -> Result<()> {
    let mut stream = tokio::time::timeout(IO, TcpStream::connect((address, 42426))).await??;
    tcp_exchange(&mut stream, &[0x33; FRAME]).await
}

async fn tcp_echo(
    listener: TcpListener,
    stop: CancellationToken,
    counts: Arc<[AtomicU64; 4]>,
    index: usize,
) -> Result<()> {
    let mut streams = JoinSet::new();
    let result: Result<()> = async {
        loop {
            tokio::select! {
                () = stop.cancelled() => return Ok(()),
                Some(result) = streams.join_next(), if !streams.is_empty() => { result??; }
                accepted = listener.accept(), if streams.len() < 8 => {
                    let (mut stream, _) = accepted?;
                    let stop = stop.clone();
                    let counts = counts.clone();
                    streams.spawn(async move {
                        let mut bytes = [0; FRAME];
                        loop {
                            tokio::select! {
                                () = stop.cancelled() => return Ok::<(), Error>(()),
                                result = stream.read_exact(&mut bytes) => match result {
                                    Ok(_) => {},
                                    Err(error) if matches!(error.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset) => return Ok(()),
                                    Err(error) => return Err(error.into()),
                                }
                            }
                            tokio::select! {
                                () = stop.cancelled() => return Ok(()),
                                result = stream.write_all(&bytes) => { result?; }
                            }
                            counts[index].fetch_add(1, Ordering::Relaxed);
                        }
                    });
                }
            }
        }
    }.await;
    stop.cancel();
    let mut cleanup = Ok(());
    while let Some(joined) = streams.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => cleanup = Err(error),
            Err(error) => cleanup = Err(error.into()),
        }
    }
    result.and(cleanup)
}

pub struct Traffic {
    udp4: UdpSocket,
    udp6: UdpSocket,
    tcp4: Option<TcpStream>,
    tcp6: Option<TcpStream>,
    destination: Addresses,
    sequence: u64,
    pub counts: Counts,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PacketEvidence {
    sockets: [SocketAddr; 4],
    counts: Counts,
    elapsed_ms: u128,
    last_echo_sequence: u64,
    echoed_frame: Vec<u8>,
}

impl Traffic {
    pub async fn new(source: &Addresses, destination: &Addresses) -> Result<Self> {
        let udp4 = UdpSocket::bind((source.ipv4, 0)).await?;
        let udp6 = UdpSocket::bind((source.ipv6, 0)).await?;
        udp4.connect((destination.ipv4, UDP)).await?;
        udp6.connect((destination.ipv6, UDP)).await?;
        Ok(Self {
            udp4,
            udp6,
            tcp4: None,
            tcp6: None,
            destination: destination.clone(),
            sequence: 0,
            counts: Counts::default(),
        })
    }
    fn frame(&mut self) -> [u8; FRAME] {
        self.sequence += 1;
        let mut frame = [0x5a; FRAME];
        frame[..8].copy_from_slice(&self.sequence.to_be_bytes());
        frame
    }
    pub async fn first_udp(&mut self) -> Result<()> {
        let deadline = Instant::now() + STARTUP;
        loop {
            let frame = self.frame();
            self.udp4.send(&frame).await?;
            if udp_receive(&self.udp4, &frame, Duration::from_millis(500)).await? {
                self.counts.udp4 += 1;
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("initial IPv4 UDP demand did not produce echo".into());
            }
        }
    }
    pub async fn add_demands(&mut self, source: &Addresses) -> Result<()> {
        // The same first UDP socket survives every later demand and snapshot.
        self.tcp4 = Some(
            tokio::time::timeout(STARTUP, TcpStream::connect((self.destination.ipv4, TCP)))
                .await??,
        );
        if self.tcp4.as_ref().ok_or("missing TCP4")?.local_addr()?.ip() != IpAddr::V4(source.ipv4) {
            return Err("TCP4 did not select managed source".into());
        }
        let deadline = Instant::now() + STARTUP;
        loop {
            let frame = self.frame();
            udp_exchange(&self.udp4, &frame).await?;
            self.counts.udp4 += 1;
            tcp_exchange(self.tcp4.as_mut().ok_or("missing TCP4")?, &frame).await?;
            self.counts.tcp4 += 1;
            self.udp6.send(&frame).await?;
            if udp_receive(&self.udp6, &frame, Duration::from_millis(500)).await? {
                self.counts.udp6 += 1;
                break;
            }
            if Instant::now() >= deadline {
                return Err("IPv6 demand did not produce echo".into());
            }
        }
        self.tcp6 = Some(
            tokio::time::timeout(STARTUP, TcpStream::connect((self.destination.ipv6, TCP)))
                .await??,
        );
        if self.tcp6.as_ref().ok_or("missing TCP6")?.local_addr()?.ip() != IpAddr::V6(source.ipv6) {
            return Err("TCP6 did not select managed source".into());
        }
        Ok(())
    }
    pub async fn exchange(&mut self) -> Result<()> {
        let frame = self.frame();
        udp_exchange(&self.udp4, &frame).await?;
        self.counts.udp4 += 1;
        udp_exchange(&self.udp6, &frame).await?;
        self.counts.udp6 += 1;
        tcp_exchange(self.tcp4.as_mut().ok_or("TCP4 not established")?, &frame).await?;
        self.counts.tcp4 += 1;
        tcp_exchange(self.tcp6.as_mut().ok_or("TCP6 not established")?, &frame).await?;
        self.counts.tcp6 += 1;
        Ok(())
    }
    pub async fn restored(&mut self) -> Result<()> {
        self.first_udp().await?;
        self.tcp4 = Some(
            tokio::time::timeout(STARTUP, TcpStream::connect((self.destination.ipv4, TCP)))
                .await??,
        );
        self.tcp6 = Some(
            tokio::time::timeout(STARTUP, TcpStream::connect((self.destination.ipv6, TCP)))
                .await??,
        );
        self.exchange().await
    }
    pub async fn blocked(&mut self) -> Result<()> {
        let frame = self.frame();
        for socket in [&self.udp4, &self.udp6] {
            match socket.send(&frame).await {
                Ok(FRAME) => {}
                Ok(_) => return Err("short blocked UDP write".into()),
                Err(error) if expected_block(&error) => {}
                Err(error) => return Err(error.into()),
            }
            if udp_receive(socket, &frame, Duration::from_secs(1)).await? {
                return Err("UDP echoed after access/lease invalidation".into());
            }
        }
        for stream in [&mut self.tcp4, &mut self.tcp6].into_iter().flatten() {
            match tokio::time::timeout(IO, stream.write_all(&frame)).await {
                Ok(Ok(())) | Err(_) => {}
                Ok(Err(error)) if expected_block(&error) => {}
                Ok(Err(error)) => return Err(error.into()),
            }
            let mut byte = [0];
            match tokio::time::timeout(Duration::from_secs(1), stream.read(&mut byte)).await {
                Err(_) | Ok(Ok(0)) => {}
                Ok(Err(error)) if expected_block(&error) => {}
                Ok(Err(error)) => return Err(error.into()),
                Ok(Ok(_)) => return Err("TCP delivered after access/lease invalidation".into()),
            }
        }
        Ok(())
    }
    pub fn sockets(&self) -> Result<[SocketAddr; 4]> {
        Ok([
            self.udp4.local_addr()?,
            self.udp6.local_addr()?,
            self.tcp4.as_ref().ok_or("missing TCP4")?.local_addr()?,
            self.tcp6.as_ref().ok_or("missing TCP6")?.local_addr()?,
        ])
    }

    pub fn evidence(&self, elapsed: Duration) -> Result<PacketEvidence> {
        let mut frame = vec![0x5a; FRAME];
        frame[..8].copy_from_slice(&self.sequence.to_be_bytes());
        Ok(PacketEvidence {
            sockets: self.sockets()?,
            counts: self.counts,
            elapsed_ms: elapsed.as_millis(),
            last_echo_sequence: self.sequence,
            echoed_frame: frame,
        })
    }
}

fn expected_block(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NetworkUnreachable
            | std::io::ErrorKind::HostUnreachable
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::TimedOut
    )
}

async fn udp_receive(
    socket: &UdpSocket,
    expected: &[u8; FRAME],
    duration: Duration,
) -> Result<bool> {
    let result = tokio::time::timeout(duration, async {
        let mut bytes = [0; FRAME + 1];
        loop {
            match socket.recv(&mut bytes).await {
                Ok(length) if length == FRAME && bytes[..length] == *expected => return Ok(true),
                Ok(FRAME) => continue, // old warmup challenge, never counts as current evidence
                Ok(_) => return Err("unexpected UDP echo length".into()),
                Err(error) if expected_block(&error) => return Ok(false),
                Err(error) => return Err(error.into()),
            }
        }
    })
    .await;
    match result {
        Ok(value) => value,
        Err(_) => Ok(false),
    }
}

async fn udp_exchange(socket: &UdpSocket, frame: &[u8; FRAME]) -> Result<()> {
    // A publication may cancel an accepted old datagram. UDP does not promise
    // delivery; prove continuity on this same socket with bounded challenges.
    let deadline = Instant::now() + IO;
    loop {
        socket.send(frame).await?;
        if udp_receive(socket, frame, Duration::from_millis(500)).await? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("persistent UDP echo unavailable".into());
        }
    }
}

async fn tcp_exchange(stream: &mut TcpStream, frame: &[u8; FRAME]) -> Result<()> {
    tokio::time::timeout(IO, async {
        stream.write_all(frame).await?;
        let mut bytes = [0; FRAME];
        stream.read_exact(&mut bytes).await?;
        if bytes != *frame {
            return Err("TCP echo challenge mismatch".into());
        }
        Ok::<(), Error>(())
    })
    .await?
}
