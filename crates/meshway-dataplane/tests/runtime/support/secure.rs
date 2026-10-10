//! Real Control identities and native TCP sessions; no synthetic identity adapter.
use meshway_dataplane::secure::{Carrier, PendingFrame};
use meshway_transport::{Receiver, Sender};
use meshway_transport_tcp::TcpTransport;
use std::{net::SocketAddr, time::Duration};
use tokio::net::{TcpListener, TcpStream};

pub const IO_WAIT: Duration = Duration::from_secs(10);
pub const SENTINEL: &[u8] = b"Meshway-real-Control-native-TCP-encrypted-UDP-payload-sentinel";

pub struct End {
    pub sender: Sender,
    pub receiver: Receiver,
    pub peer: SocketAddr,
}
impl End {
    fn adopt(stream: TcpStream) -> Self {
        let transport =
            TcpTransport::adopt(stream, &Default::default(), &Default::default()).unwrap();
        let peer = transport.peer_addr().unwrap();
        let (sender, receiver) = transport
            .into_session(meshway_transport::Limits::new(128 * 1024, 8, 1024 * 1024, 8, 1).unwrap())
            .unwrap()
            .into_split();
        Self {
            sender,
            receiver,
            peer,
        }
    }
    pub async fn send(&mut self, pending: PendingFrame) {
        tokio::time::timeout(
            IO_WAIT,
            meshway_dataplane::carrier::send(&mut self.sender, pending, Some(self.peer)),
        )
        .await
        .unwrap()
        .unwrap();
    }
    pub async fn recv(&mut self) -> Vec<u8> {
        let frame = tokio::time::timeout(IO_WAIT, self.receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.peer, Some(self.peer));
        assert!(
            !frame
                .payload
                .windows(SENTINEL.len())
                .any(|part| part == SENTINEL)
        );
        frame.payload.into_vec()
    }
    pub async fn close(&mut self) {
        tokio::time::timeout(IO_WAIT, self.sender.shutdown())
            .await
            .unwrap()
            .unwrap();
    }
}

pub struct Link {
    pub dial: End,
    pub listen: End,
    pub carrier: Carrier,
}
impl Link {
    pub async fn connect(listener: TcpListener, carrier: Carrier) -> Self {
        let address = listener.local_addr().unwrap();
        let (dial, listen) = tokio::time::timeout(IO_WAIT, async {
            tokio::join!(TcpStream::connect(address), listener.accept())
        })
        .await
        .unwrap();
        Self {
            dial: End::adopt(dial.unwrap()),
            listen: End::adopt(listen.unwrap().0),
            carrier,
        }
    }
}
