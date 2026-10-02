//! A generic owning stream is not protection proof.
use meshway_agent_connections::sockets::{OuterTcp, Protection, Purpose};
use meshway_dataplane::sockets::{NativeSocketFactory, SystemSocketFactory};
use std::{io, net::SocketAddr};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

struct RejectUnprotected;
impl Protection for RejectUnprotected {
    fn prepare(&self, _: SocketAddr, _: Purpose<'_>) -> io::Result<TcpSocket> {
        Err(io::Error::other("unprotected socket"))
    }
    fn accepted(&self, _: &TcpStream, _: &str) -> io::Result<()> {
        Err(io::Error::other("unprotected socket"))
    }
    fn native(&self) -> &dyn NativeSocketFactory {
        &SystemSocketFactory
    }
}

#[tokio::test]
async fn boxing_plain_tcp_must_not_skip_protection_acceptance() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    assert!(RejectUnprotected.accepted(&accepted, "listener").is_err());
    let boxed = OuterTcp::Owned(Box::new(accepted));
    assert!(boxed.accepted(&RejectUnprotected, "listener").is_err());
    drop(client);
}
