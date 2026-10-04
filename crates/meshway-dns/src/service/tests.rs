use super::*;
use crate::{DispatchError, DnsFuture, PayloadCodec};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query as Question},
    rr::{Name, RecordType},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::{io::AsyncWriteExt, sync::Notify};

struct Handler {
    hang: bool,
    active: AtomicUsize,
    entered: Notify,
    stopped: AtomicBool,
}

impl Handler {
    fn new(hang: bool) -> Arc<Self> {
        Arc::new(Self {
            hang,
            active: AtomicUsize::new(0),
            entered: Notify::new(),
            stopped: AtomicBool::new(false),
        })
    }
}

impl Dispatcher for Handler {
    fn dispatch(
        &self,
        _query: Query,
        _deadline: Instant,
    ) -> DnsFuture<'_, Result<Answer, DispatchError>> {
        Box::pin(async {
            struct Active<'a>(&'a AtomicUsize);
            impl Drop for Active<'_> {
                fn drop(&mut self) {
                    self.0.fetch_sub(1, Ordering::SeqCst);
                }
            }
            self.active.fetch_add(1, Ordering::SeqCst);
            let _active = Active(&self.active);
            self.entered.notify_one();
            if self.hang {
                std::future::pending::<()>().await;
            }
            Err(DispatchError::Gateway)
        })
    }
    fn stop(&self) -> DnsFuture<'_, ()> {
        Box::pin(async {
            self.stopped.store(true, Ordering::SeqCst);
        })
    }
}

fn query(id: u16) -> Bytes {
    let mut query = Message::new(id, MessageType::Query, OpCode::Query);
    query.add_query(Question::query(
        Name::from_ascii("node.mesh.example.").unwrap(),
        RecordType::A,
    ));
    Bytes::from(query.to_vec().unwrap())
}

async fn listeners() -> (UdpSocket, TcpListener, SocketAddr) {
    let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = tcp.local_addr().unwrap();
    let udp = UdpSocket::bind(address).await.unwrap();
    (udp, tcp, address)
}

fn check_failure(bytes: &[u8], id: u16) {
    let response = Message::from_vec(bytes).unwrap();
    assert_eq!(response.metadata.id, id);
    assert_eq!(response.metadata.response_code, ResponseCode::ServFail);
    assert!(!response.metadata.authoritative && !response.metadata.authentic_data);
    assert!(response.answers.is_empty() && response.authorities.is_empty());
}

async fn response(
    stream: &mut TcpStream,
    codec: &mut PayloadCodec,
    buffer: &mut BytesMut,
) -> BytesMut {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(frame) = codec.decode(buffer).unwrap() {
                return frame;
            }
            assert_ne!(stream.read_buf(buffer).await.unwrap(), 0);
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_udp_and_pipelined_tcp_share_dispatcher_and_return_only_servfail_on_outage() {
    let handler = Handler::new(false);
    let (udp, tcp, address) = listeners().await;
    let shutdown = CancellationToken::new();
    let service = Service::new(handler.clone(), ServiceLimits::default()).unwrap();
    let task = tokio::spawn(service.run(udp, tcp, shutdown.clone()));
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.send_to(&query(1), address).await.unwrap();
    let mut buffer = [0u8; 512];
    let (count, peer) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(peer, address);
    check_failure(&buffer[..count], 1);

    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut codec = PayloadCodec::new(u16::MAX).unwrap();
    let mut framed = BytesMut::new();
    codec.encode(query(2), &mut framed).unwrap();
    codec.encode(query(3), &mut framed).unwrap();
    stream.write_all(&framed).await.unwrap();
    let mut received = BytesMut::new();
    check_failure(&response(&mut stream, &mut codec, &mut received).await, 2);
    check_failure(&response(&mut stream, &mut codec, &mut received).await, 3);
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(handler.stopped.load(Ordering::SeqCst));
    assert_eq!(handler.active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn timeout_sends_servfail_inside_original_lifetime_and_shutdown_joins_work() {
    let handler = Handler::new(true);
    let (udp, tcp, address) = listeners().await;
    let shutdown = CancellationToken::new();
    let limits = ServiceLimits {
        query_timeout: Duration::from_millis(500),
        max_queries: 1,
        ..ServiceLimits::default()
    };
    let service = Service::new(handler.clone(), limits).unwrap();
    let task = tokio::spawn(service.run(udp, tcp, shutdown.clone()));
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    client.send_to(&query(1), address).await.unwrap();
    let mut buffer = [0u8; 512];
    let (count, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    check_failure(&buffer[..count], 1);
    assert_eq!(handler.active.load(Ordering::SeqCst), 0);
    // Consume the first entry notification, then stop a second active request.
    handler.entered.notified().await;
    client.send_to(&query(2), address).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), handler.entered.notified())
        .await
        .unwrap();
    assert_eq!(handler.active.load(Ordering::SeqCst), 1);
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(handler.active.load(Ordering::SeqCst), 0);
    assert!(handler.stopped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn wildcard_bind_is_rejected_before_accepting_queries() {
    let tcp = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let udp = UdpSocket::bind(tcp.local_addr().unwrap()).await.unwrap();
    let service = Service::new(Handler::new(false), ServiceLimits::default()).unwrap();
    let error = service
        .run(udp, tcp, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}
