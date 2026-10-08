use super::*;
use crate::{AcceptedSocket, DatagramSocket, ListenerSocket};
use futures_util::{future::BoxFuture, task::AtomicWaker};
use meshway_authority::ExecutionSnapshot;
use meshway_transport::raw::{Datagram, DatagramRead};
use meshway_transport_spec::{DialParameters, Listener};
use meshway_transport_udp::UdpTransport;
use std::{
    collections::BTreeSet,
    io,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll},
    time::Duration,
};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

#[cfg(target_os = "linux")]
mod linux_resources;
mod reattachment;
mod regression;
mod resources;
mod streaming;
pub(super) mod support;
mod traversal;

#[derive(Default)]
struct Faults {
    drops: Mutex<BTreeSet<u8>>,
    held_controls: Mutex<BTreeSet<u8>>,
    blackhole: AtomicBool,
    block_data: AtomicBool,
    blocked: AtomicBool,
    writes: AtomicUsize,
    waker: AtomicWaker,
    controls: Mutex<Vec<(u8, SocketAddr, Vec<u8>)>>,
}

struct Endpoint {
    inner: UdpTransport,
    faults: Arc<Faults>,
}
impl Datagram for Endpoint {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        self.inner.poll_recv_from(cx, bytes)
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        let control = bytes.starts_with(adjacency::MAGIC);
        if !control && self.faults.block_data.load(Ordering::SeqCst) {
            self.faults.waker.register(cx.waker());
            self.faults.blocked.store(true, Ordering::SeqCst);
            return Poll::Pending;
        }
        if self.faults.blackhole.load(Ordering::SeqCst) {
            return Poll::Ready(Ok(bytes.len()));
        }
        if control {
            let kind = bytes[8];
            let mut recorded = self.faults.controls.lock().unwrap();
            if recorded.len() < 128 {
                recorded.push((kind, peer, bytes.to_vec()));
            }
            if self.faults.held_controls.lock().unwrap().contains(&kind) {
                return Poll::Ready(Ok(bytes.len()));
            }
            if self.faults.drops.lock().unwrap().remove(&kind) {
                return Poll::Ready(Ok(bytes.len()));
            }
        }
        let result = self.inner.poll_send_to(cx, bytes, peer);
        if !control && matches!(result, Poll::Ready(Ok(_))) {
            self.faults.writes.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

struct Factory {
    udp: Mutex<BTreeMap<SocketAddr, UdpSocket>>,
    tcp: Mutex<BTreeMap<SocketAddr, TcpListener>>,
    faults: Arc<Faults>,
    blocked_dialers: Mutex<BTreeSet<NodeId>>,
    acquisitions: AtomicUsize,
}
impl Factory {
    fn new(fixture: &mut support::Fixture, faults: Arc<Faults>) -> Arc<Self> {
        Arc::new(Self {
            udp: Mutex::new(
                [fixture.udp.take(), fixture.reverse_udp.take()]
                    .into_iter()
                    .flatten()
                    .chain(fixture.extra_udp.drain(..))
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            tcp: Mutex::new(
                fixture
                    .tcp
                    .take()
                    .into_iter()
                    .chain(fixture.extra_tcp.drain(..))
                    .map(|socket| (socket.local_addr().unwrap(), socket))
                    .collect(),
            ),
            faults,
            blocked_dialers: Mutex::new(BTreeSet::new()),
            acquisitions: AtomicUsize::new(0),
        })
    }
    fn endpoint(&self, socket: UdpSocket) -> Box<dyn Datagram> {
        Box::new(Endpoint {
            inner: UdpTransport::adopt(socket, &Default::default()).unwrap(),
            faults: self.faults.clone(),
        })
    }
}
struct StreamListener(TcpListener);
impl ListenerSocket for StreamListener {
    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.0.local_addr()?)
    }
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>> {
        Box::pin(async {
            let (stream, peer) = self.0.accept().await?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer,
                stream: Box::new(stream),
            })
        })
    }
}
impl SocketFactory for Factory {
    fn connect<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let candidate = &cfg.dialers[key];
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node_id) {
                return Err(Error::Unavailable);
            }
            assert!(!matches!(
                candidate.parameters,
                DialParameters::NativeUdp { .. }
            ));
            let endpoint = &candidate.endpoints[index];
            let stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port.get())).await?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer: stream.peer_addr()?,
                stream: Box::new(stream),
            })
        })
    }
    fn listen<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move {
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let address = streaming::listener_address(cfg.listeners[key].declaration())?;
            let reserved = self.tcp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => TcpListener::bind(address).await?,
            };
            assert_eq!(socket.local_addr()?.port(), address.port());
            Ok(Box::new(StreamListener(socket)) as Box<dyn ListenerSocket>)
        })
    }
    fn datagram_listener<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            let Listener::NativeUdp(listener) = cfg.listeners[key].declaration() else {
                return Err(Error::Configuration);
            };
            let address = SocketAddr::new(listener.listen, listener.listen_port.get());
            let reserved = self.udp.lock().unwrap().remove(&address);
            let socket = match reserved {
                Some(socket) => socket,
                None => UdpSocket::bind(address).await?,
            };
            assert_eq!(socket.local_addr()?.port(), listener.listen_port.get());
            Ok(self.endpoint(socket))
        })
    }
    fn datagram_candidate<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        index: usize,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            self.acquisitions.fetch_add(1, Ordering::SeqCst);
            let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
            if self.blocked_dialers.lock().unwrap().contains(&cfg.node_id) {
                return Err(Error::Unavailable);
            }
            let candidate = &cfg.dialers[key];
            assert!(matches!(
                candidate.parameters,
                DialParameters::NativeUdp { .. }
            ));
            let endpoint = &candidate.endpoints[index];
            let peer = SocketAddr::new(endpoint.host.parse().unwrap(), endpoint.port.get());
            let local = if peer.is_ipv4() {
                "127.0.0.1:0"
            } else {
                "[::1]:0"
            };
            Ok(vec![DatagramSocket {
                endpoint: self.endpoint(UdpSocket::bind(local).await?),
                targets: vec![peer],
            }])
        })
    }
}

async fn linked(manager: &Manager, count: usize, protocol: TransportProtocol) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let facts = manager.snapshot().unwrap();
            if facts.links.len() == count
                && facts
                    .links
                    .iter()
                    .all(|link| link.carrier.protocol == protocol)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded actual peer handshake/failover");
}

async fn frame(manager: &mut Manager) -> (LinkFact, Frame) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match manager.recv().await {
                Event::Frame { link, frame, .. } => return (*link, frame),
                Event::Changed => {}
                Event::Closed => panic!("manager closed"),
            }
        }
    })
    .await
    .unwrap()
}

mod handshake;
mod receipts;
mod receive;
mod scope;

// Stable entry names are selected by the fast profile real-time exclusions.
#[tokio::test]
async fn udp_and_stream_receive_timestamp_precedes_delayed_consumer_poll() {
    receive::udp_and_stream_receive_timestamp_precedes_delayed_consumer_poll().await;
}

#[tokio::test]
async fn real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner() {
    handshake::real_ipv6_udp_blackhole_selects_allowed_stream_without_replacing_secure_owner()
        .await;
}

#[tokio::test]
async fn simultaneous_configured_initiators_converge_on_one_logical_carrier() {
    handshake::simultaneous_configured_initiators_converge_on_one_logical_carrier().await;
}
