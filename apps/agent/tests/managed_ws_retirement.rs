#![cfg(feature = "managed-connections")]
// Retains the independent Astra complete-chain counterexample, including
// delayed EOF and a gate below tungstenite's dirty body buffer.
use meshway_dataplane::{
    Identity, LabConfig, PeerConfig, PrivateKey, Runtime, StreamRole, TransportKind,
    packet_io::{self, Ipv4PacketPlan, PacketBatch},
};
use meshway_transport_vless::websocket;
use std::{
    collections::BTreeMap,
    future::{Future, poll_fn},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf},
    sync::watch,
    time::{Instant, timeout},
};
#[derive(Default)]
struct Gate {
    blocked: AtomicBool,
    allowance: AtomicUsize,
    retired: AtomicBool,
    late_bytes: AtomicUsize,
    attempted: AtomicBool,
    waker: Mutex<Option<Waker>>,
}
struct Gated<S> {
    stream: S,
    gate: Arc<Gate>,
}
// Delay peer-visible EOF within this bounded test, like data arriving before FIN.
// This prevents immediate in-memory EOF unregistering B before dispatch gets CPU.
struct DelayEof(DuplexStream);
impl AsyncRead for DelayEof {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.0).poll_read(cx, buf);
        if matches!(result, Poll::Ready(Ok(())))
            && buf.filled().len() == before
            && buf.remaining() > 0
        {
            Poll::Pending
        } else {
            result
        }
    }
}
impl AsyncWrite for DelayEof {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for Gated<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Gated<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.gate.blocked.load(Ordering::SeqCst) {
            let left = self.gate.allowance.load(Ordering::SeqCst);
            if left > 0 {
                let n = buf.len().min(left);
                let result = Pin::new(&mut self.stream).poll_write(cx, &buf[..n]);
                if let Poll::Ready(Ok(n)) = &result {
                    self.gate.allowance.fetch_sub(*n, Ordering::SeqCst);
                }
                return result;
            }
            self.gate.attempted.store(true, Ordering::SeqCst);
            *self.gate.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        } else {
            let result = Pin::new(&mut self.stream).poll_write(cx, buf);
            if self.gate.retired.load(Ordering::SeqCst)
                && let Poll::Ready(Ok(n)) = &result
            {
                self.gate.late_bytes.fetch_add(*n, Ordering::SeqCst);
            }
            result
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
fn config(n: &str, i: &Identity, p: &str, j: &Identity) -> LabConfig {
    LabConfig {
        node_id: n.into(),
        private_key: PrivateKey::from_hex(&i.private_key_hex()).unwrap(),
        public_key: i.public_key_hex(),
        trusted_peers: vec![PeerConfig {
            node_id: p.into(),
            public_key: j.public_key_hex(),
            transports: vec![TransportKind::Vless],
        }],
        allowed_paths: vec![vec!["a".into(), "b".into()]],
        listeners: vec![],
        outgoing: vec![],
        limits: Default::default(),
    }
}
fn packet(marker: u8) -> Vec<u8> {
    let mut p = vec![0; 21];
    p[0] = 0x45;
    p[3] = 21;
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&[100, 96, 0, 1]);
    p[16..20].copy_from_slice(&[100, 96, 0, 2]);
    p[20] = marker;
    let mut sum: u32 = p[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_be_bytes([b[0], b[1]]) as u32)
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    p[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
    p
}
#[tokio::test(flavor = "current_thread")]
async fn complete_chain_dirty_ws_buffer_never_writes_after_retirement() {
    let mut leaks = 0;
    for _ in 0..20 {
        if run().await {
            leaks += 1;
        }
    }
    println!("TLS/WS/VLESS/Noise after-retirement writes or deliveries: {leaks}/20");
    assert_eq!(leaks, 0, "WS read-side flush bypasses retired writer fence");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_retirement_closes_dirty_carrier_and_fresh_carrier_recovers() {
    for _ in 0..20 {
        assert!(!run().await);
    }
}

async fn attach(a: &Runtime, b: &Runtime, gate: Arc<Gate>, priority: u16) {
    let (sa, sb) = tokio::io::duplex(65536);
    let params = websocket::Parameters {
        host: "mesh.test".into(),
        path: "/mesh".into(),
    };
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["mesh.test".into()]).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let (tc, ts) = tokio::join!(
        connector.connect(
            rustls::pki_types::ServerName::try_from("mesh.test").unwrap(),
            sa
        ),
        acceptor.accept(DelayEof(sb))
    );
    let (tc, ts) = (tc.unwrap(), ts.unwrap());
    let (c, s) = tokio::join!(
        websocket::connect(
            Gated {
                stream: tc,
                gate: gate.clone()
            },
            &params
        ),
        websocket::accept(ts, &params)
    );
    let (c, mut s) = (c.unwrap(), s.unwrap());
    let uuid: meshway_transport_vless::SecretUuid =
        "b590664f-5111-4c8b-a481-82594488df9e".parse().unwrap();
    let target =
        meshway_transport_vless::Target::ip(std::net::Ipv4Addr::LOCALHOST.into(), 8421).unwrap();
    let vc = meshway_transport_vless::ClientConfig {
        uuid: uuid.clone(),
        target: target.clone(),
    };
    let vs = meshway_transport_vless::ServerConfig {
        uuid,
        expected_target: target,
    };
    let (c, accepted) = tokio::join!(
        meshway_transport_vless::client_stream(c, &vc),
        meshway_transport_vless::server_handshake(&mut s, &vs)
    );
    let c = c.unwrap();
    accepted.unwrap();
    let (ra, rb) = tokio::join!(
        a.attach_stream_with_context(
            c,
            StreamRole::Dialer { peer: "b".into() },
            TransportKind::Vless,
            priority,
            Default::default()
        ),
        b.attach_stream_with_context(
            s,
            StreamRole::Acceptor,
            TransportKind::Vless,
            priority,
            Default::default()
        )
    );
    ra.unwrap();
    rb.unwrap();
}

async fn run() -> bool {
    let ia = Identity::generate().unwrap();
    let ib = Identity::generate().unwrap();
    let a = Arc::new(Runtime::start(config("a", &ia, "b", &ib)).await.unwrap());
    let b = Runtime::start(config("b", &ib, "a", &ia)).await.unwrap();
    let mut recv = b.take_packet_receiver().unwrap();
    let gate = Arc::new(Gate::default());
    attach(&a, &b, gate.clone(), 10).await;
    let (device, input) = packet_io::channel(8).unwrap();
    let (_lease, rx) = watch::channel(Some(Instant::now() + Duration::from_secs(20)));
    let plan = Ipv4PacketPlan::new(
        "100.96.0.1".parse().unwrap(),
        1000,
        BTreeMap::from([("100.96.0.2".parse().unwrap(), vec!["a".into(), "b".into()])]),
        BTreeMap::new(),
    );
    let mut pump = Box::pin(packet_io::run(device.clone(), a.clone(), plan, rx));
    input
        .try_feed(PacketBatch::new(vec![packet(1)]).unwrap())
        .unwrap();
    tokio::select! {r=&mut pump=>panic!("unexpected pump end {r:?}"),p=timeout(Duration::from_secs(2),recv.recv())=>assert_eq!(p.unwrap().unwrap().payload[20],1)}
    // Allow the first masked WS frame containing the 4-byte record length. Then
    // block the complete encrypted-record body buffered by start_send.
    gate.allowance.store(10, Ordering::SeqCst);
    gate.blocked.store(true, Ordering::SeqCst);
    input
        .try_feed(PacketBatch::new(vec![packet(42)]).unwrap())
        .unwrap();
    tokio::select! {r=&mut pump=>panic!("unexpected pump end {r:?}"),_=timeout(Duration::from_secs(2),async{while !gate.attempted.load(Ordering::SeqCst){tokio::task::yield_now().await;}})=>{}}
    assert!(gate.attempted.load(Ordering::SeqCst));
    // Synchronous drop retires the fence. Do not yield to the carrier task before
    // successor activation and unblocking: this is a legal executor scheduling.
    drop(pump);
    packet_io::Device::drain(device.as_ref()).unwrap();
    let (_lease2, rx) = watch::channel(Some(Instant::now() + Duration::from_secs(20)));
    let active = AtomicBool::new(false);
    let empty = Ipv4PacketPlan::new(
        "100.96.0.1".parse().unwrap(),
        1000,
        BTreeMap::new(),
        BTreeMap::new(),
    );
    let mut next = Box::pin(packet_io::run_with_activation(
        device,
        a.clone(),
        empty,
        rx,
        || active.store(true, Ordering::SeqCst),
    ));
    poll_fn(|cx| {
        assert!(next.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(active.load(Ordering::SeqCst));
    gate.retired.store(true, Ordering::SeqCst);
    gate.blocked.store(false, Ordering::SeqCst);
    if let Some(w) = gate.waker.lock().unwrap().take() {
        w.wake();
    }
    let leak = matches!(timeout(Duration::from_millis(75),recv.recv()).await,Ok(Some(p)) if p.payload[20]==42);
    let late = gate.late_bytes.load(Ordering::SeqCst);
    if late > 0 {
        println!("underlying old carrier wrote {late} bytes after retirement; delivered={leak}");
    }
    drop(next);
    // The partial carrier is discarded. Re-authenticate a genuinely new full
    // chain on the SAME runtimes (no E2E/nonce cache reset), and deliver again.
    // Delayed EOF intentionally keeps the remote old channel listed. The
    // documented authenticated priority replacement retires it, rather than
    // clearing runtimes/caches or weakening channel admission for this test.
    attach(&a, &b, Arc::new(Gate::default()), 5).await;
    let (device, input) = packet_io::channel(8).unwrap();
    let (_lease, rx) = watch::channel(Some(Instant::now() + Duration::from_secs(20)));
    let plan = Ipv4PacketPlan::new(
        "100.96.0.1".parse().unwrap(),
        1000,
        BTreeMap::from([("100.96.0.2".parse().unwrap(), vec!["a".into(), "b".into()])]),
        BTreeMap::new(),
    );
    let mut recovered = Box::pin(packet_io::run(device, a.clone(), plan, rx));
    input
        .try_feed(PacketBatch::new(vec![packet(43)]).unwrap())
        .unwrap();
    tokio::select! {r=&mut recovered=>panic!("recovered pump ended {r:?}"),p=timeout(Duration::from_secs(2),recv.recv())=>assert_eq!(p.unwrap().unwrap().payload[20],43)}
    drop(recovered);
    a.shutdown().await;
    b.shutdown().await;
    leak || late > 0
}
