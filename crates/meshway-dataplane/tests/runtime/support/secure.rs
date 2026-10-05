//! Real Control identities and native TCP sessions; no synthetic identity adapter.
use super::{Mesh, Node, Server, request};
use meshway_configuration::{AccessRule, MeshRoute, execution::*};
use meshway_dataplane::secure::{Carrier, PendingFrame, Received, SecureNode};
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_protocol::DemandRequest;
use meshway_transport::{Receiver, Sender};
use meshway_transport_spec::TransportProtocol;
use meshway_transport_tcp::TcpTransport;
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
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
        frame.payload
    }
    async fn close(&mut self) {
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
    async fn connect(listener: TcpListener, carrier: Carrier) -> Self {
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

pub struct SecureMesh {
    pub mesh: Mesh,
    pub a: SecureNode,
    pub r: SecureNode,
    pub b: SecureNode,
    pub ar: Link,
    pub rb: Link,
}
impl SecureMesh {
    pub async fn reconcile(&mut self) {
        for (node, secure) in [
            (&mut self.mesh.a, &mut self.a),
            (&mut self.mesh.r, &mut self.r),
            (&mut self.mesh.b, &mut self.b),
        ] {
            let snapshot = node.sync_current().await;
            secure
                .reconcile(node.control.dataplane_identity(snapshot).unwrap())
                .unwrap();
        }
    }

    pub async fn renew(&mut self) {
        let lease = self.mesh.a.control.renew_instance().await.unwrap();
        self.mesh
            .a
            .control
            .renew_demand(&meshway_protocol::RenewDemandRequest {
                instance: self.mesh.a.instance(),
                request_id: self.mesh.demand,
                expires_at_ms: lease.expires_at_ms,
            })
            .await
            .unwrap();
        self.reconcile().await;
    }

    pub async fn start() -> Self {
        let server = Server::start().await;
        let owner = server.owner().await;
        let caps = || {
            BTreeMap::from([(
                "transport:native_tcp".into(),
                Capability::Transport {
                    roles: vec![TransportRole::Dial, TransportRole::Listen],
                    streams: vec![StreamKind::Tcp],
                },
            )])
        };
        let mut a = server.enroll_with_capabilities(&owner, "a", caps()).await;
        let mut r = server.enroll_with_capabilities(&owner, "r", caps()).await;
        let mut b = server.enroll_with_capabilities(&owner, "b", caps()).await;
        let r_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let b_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        server.configure(&owner, |input| {
            for (id, address) in [(r.id, r_listener.local_addr().unwrap()), (b.id, b_listener.local_addr().unwrap())] {
                input.nodes.get_mut(&id).unwrap().listeners = serde_json::from_value(serde_json::json!({
                    "tcp": {"type":"native_tcp", "listen":"127.0.0.1", "listenPort":address.port(),
                    "endpoints":[{"host":"127.0.0.1", "port":address.port()}]}
                })).unwrap();
            }
            input.access = vec![AccessRule { remark: String::new(), src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)], ip: vec!["udp:42424".into()], bidirectional: false }];
            input.routing.mesh = vec![MeshRoute { remark: String::new(), src: vec![format!("node:{}", a.id)],
                dst: vec![format!("node:{}", b.id)], via: vec![format!("node:{}", r.id)] }];
        }).await;
        let demand = request();
        let response = a
            .control
            .create_demand(&DemandRequest {
                instance: a.instance(),
                request_id: demand,
                target: BusinessTarget::Mesh { node: b.id },
                protocol: BusinessProtocol::Udp,
                port: Some(42424.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 120,
            })
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
        let path = vec![a.id, r.id, b.id];
        for node in [&mut a, &mut r, &mut b] {
            let snapshot = node.sync_current().await;
            assert_eq!(
                snapshot.configuration().unwrap().permits[&demand].paths[0].nodes,
                path
            );
        }
        let ar_carrier = Carrier {
            dialer: a.id,
            listener: r.id,
            listener_key: "tcp".into(),
            protocol: TransportProtocol::NativeTcp,
        };
        let rb_carrier = Carrier {
            dialer: r.id,
            listener: b.id,
            listener_key: "tcp".into(),
            protocol: TransportProtocol::NativeTcp,
        };
        let secure = |node: &Node, carriers| {
            SecureNode::new(
                node.control
                    .dataplane_identity(node.control.execution_snapshot().unwrap())
                    .unwrap(),
                Default::default(),
                carriers,
            )
            .unwrap()
        };
        let source = secure(&a, BTreeMap::from([(r.id, ar_carrier.clone())]));
        let relay = secure(
            &r,
            BTreeMap::from([(a.id, ar_carrier.clone()), (b.id, rb_carrier.clone())]),
        );
        let terminal = secure(&b, BTreeMap::from([(r.id, rb_carrier.clone())]));
        Self {
            mesh: Mesh {
                server,
                owner,
                a,
                r,
                b,
                demand,
                path,
            },
            a: source,
            r: relay,
            b: terminal,
            ar: Link::connect(r_listener, ar_carrier).await,
            rb: Link::connect(b_listener, rb_carrier).await,
        }
    }
    pub async fn handshake(&mut self) -> [u8; 32] {
        let first = self
            .a
            .start(self.mesh.demand, self.mesh.path.clone())
            .unwrap();
        let circuit = first.circuit();
        self.ar.dial.send(first).await;
        let first = self.ar.listen.recv().await;
        let Received::Forward(first) = self.r.receive(&self.ar.carrier, first).unwrap() else {
            panic!("relay must stay opaque")
        };
        self.rb.dial.send(first).await;
        let first = self.rb.listen.recv().await;
        let Received::Established {
            circuit: terminal,
            response: Some(second),
        } = self.b.receive(&self.rb.carrier, first).unwrap()
        else {
            panic!("terminal handshake")
        };
        assert_eq!(terminal, circuit);
        self.rb.listen.send(second).await;
        let second = self.rb.dial.recv().await;
        let Received::Forward(second) = self.r.receive(&self.rb.carrier, second).unwrap() else {
            panic!("relay must stay opaque")
        };
        self.ar.listen.send(second).await;
        let second = self.ar.dial.recv().await;
        assert!(
            matches!(self.a.receive(&self.ar.carrier, second).unwrap(), Received::Established { circuit: source, response: None } if source == circuit)
        );
        circuit
    }
    pub async fn close(mut self) {
        self.ar.dial.close().await;
        self.ar.listen.close().await;
        self.rb.dial.close().await;
        self.rb.listen.close().await;
        self.mesh.close().await;
    }
}
