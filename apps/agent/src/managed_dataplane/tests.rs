use super::*;
use futures_util::future::BoxFuture;
use meshway_dataplane::{Identity, PrivateKey};
use meshway_protocol::*;
use std::{io, sync::Arc};
use tokio::sync::{Mutex, Semaphore, mpsc};

pub(crate) fn snapshot(payload: NodeDataplane) -> AgentDataplaneSnapshot {
    AgentDataplaneSnapshot {
        digest: dataplane_digest(&payload).unwrap(),
        payload,
    }
}

pub(crate) fn fixtures(count: usize) -> (Vec<NodeDataplane>, Vec<PrivateKey>) {
    let nodes: Vec<_> = (1..=count)
        .map(|index| MeshNode {
            id: format!("{index:064x}"),
            name: format!("node-{index}"),
            country: Some("US".into()),
            mesh_ipv4: format!("100.64.0.{index}"),
            mesh_ipv6: format!("fd42::{index:x}"),
        })
        .collect();
    let keys: Vec<_> = (0..count).map(|_| Identity::generate().unwrap()).collect();
    let path: Vec<_> = nodes.iter().map(|node| node.id.clone()).collect();
    let mut reverse = path.clone();
    reverse.reverse();
    let policy = RoutingPolicy {
        default_action: DefaultRouteAction::Reject,
        rules: [path.clone(), reverse.clone()]
            .into_iter()
            .enumerate()
            .map(|(index, path)| RouteRule {
                id: format!("route-{index}"),
                source: SourceSelector {
                    node_ids: vec![path[0].clone()],
                    countries: vec![],
                },
                destination_scope: DestinationScope::Mesh,
                destination_any: vec![DestinationSelector {
                    kind: DestinationSelectorKind::Node,
                    value: path.last().unwrap().clone(),
                }],
                action: RouteAction {
                    kind: RouteActionKind::Route,
                    via: path[1..path.len() - 1].to_vec(),
                    exit_node_id: None,
                },
            })
            .collect(),
    };
    let access = MeshAccessPolicy {
        grants: [path, reverse]
            .into_iter()
            .enumerate()
            .map(|(index, path)| MeshAccessGrant {
                id: format!("grant-{index}"),
                path,
            })
            .collect(),
    };
    let ports: Vec<_> = (0..count)
        .map(|_| crate::managed_connections::tests::listener_port(ConnectionTransport::NativeUdp))
        .collect();
    let payloads = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let peer = |other: usize| ConnectionPeer {
                node_id: nodes[other].id.clone(),
                public_key: keys[other].public_key_hex(),
            };
            let endpoint = |other: usize| ConnectionEndpoint {
                host: "127.0.0.1".into(),
                port: ports[other],
            };
            let connections = NodeConnections {
                schema_version: 1,
                revision: 7,
                node_id: node.id.clone(),
                public_key: Some(keys[index].public_key_hex()),
                capabilities: crate::compiled_connection_capabilities(),
                valid: true,
                diagnostics: vec![],
                peers: (0..count)
                    .filter(|other| other.abs_diff(index) == 1)
                    .map(peer)
                    .collect(),
                listeners: if index + 1 < count {
                    vec![ConnectionListener {
                        id: format!("listen-{index}"),
                        node_id: node.id.clone(),
                        transport: ConnectionTransport::NativeUdp,
                        bind: format!("127.0.0.1:{}", ports[index]),
                        advertised_endpoints: vec![endpoint(index)],
                        credential_ref: None,
                    }]
                } else {
                    vec![]
                },
                incoming: if index + 1 < count {
                    vec![IncomingConnectionApproval {
                        candidate_id: format!("dial-{}", index + 1),
                        listener_id: format!("listen-{index}"),
                        peer_node_id: nodes[index + 1].id.clone(),
                        transport: ConnectionTransport::NativeUdp,
                    }]
                } else {
                    vec![]
                },
                outgoing: if index > 0 {
                    vec![OutgoingConnectionCandidate {
                        id: format!("dial-{index}"),
                        listener_id: format!("listen-{}", index - 1),
                        peer_node_id: nodes[index - 1].id.clone(),
                        transport: ConnectionTransport::NativeUdp,
                        endpoint: endpoint(index - 1),
                        priority: 0,
                        credential_ref: None,
                    }]
                } else {
                    vec![]
                },
            };
            NodeDataplane {
                schema_version: 1,
                revision: 7,
                node_id: node.id.clone(),
                configuration: NodeConfiguration {
                    schema_version: 1,
                    revision: 7,
                    network: MeshNetwork {
                        ipv4_prefix: "100.64.0.0/24".into(),
                        ipv6_prefix: "fd42::/64".into(),
                    },
                    node: node.clone(),
                    peers: nodes
                        .iter()
                        .filter(|other| other.id != node.id)
                        .cloned()
                        .collect(),
                    routing_policy: policy.clone(),
                    policy_valid: true,
                    diagnostics: vec![],
                },
                connections,
                access_policy: access.clone(),
                access_valid: true,
                identities: (0..count)
                    .filter(|other| *other != index)
                    .map(peer)
                    .collect(),
            }
        })
        .collect();
    (
        payloads,
        keys.into_iter()
            .map(|key| PrivateKey::from_hex(&key.private_key_hex()).unwrap())
            .collect(),
    )
}

pub(crate) fn packet(source: &str, destination: &str, protocol: u8) -> Vec<u8> {
    let mut bytes = vec![0; 28];
    bytes[0] = 0x45;
    bytes[2..4].copy_from_slice(&28_u16.to_be_bytes());
    bytes[6] = 0x40;
    bytes[8] = 64;
    bytes[9] = protocol;
    bytes[12..16].copy_from_slice(&source.parse::<Ipv4Addr>().unwrap().octets());
    bytes[16..20].copy_from_slice(&destination.parse::<Ipv4Addr>().unwrap().octets());
    checksum(&mut bytes);
    bytes
}

pub(crate) fn checksum(bytes: &mut [u8]) {
    bytes[10..12].fill(0);
    let mut sum: u32 = bytes[..20]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|word| u32::from(u16::from_be_bytes(*word)))
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    bytes[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
}

pub(crate) struct MockDevice {
    from_kernel: Mutex<mpsc::Receiver<Vec<u8>>>,
    to_kernel: mpsc::Sender<Vec<u8>>,
    pub inject_gate: Semaphore,
    pub attempted: tokio::sync::Notify,
    pub failed: std::sync::atomic::AtomicBool,
}

impl MockDevice {
    pub fn new() -> (Arc<Self>, mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>) {
        let (send, from_kernel) = mpsc::channel(32);
        let (to_kernel, receive) = mpsc::channel(32);
        (
            Arc::new(Self {
                from_kernel: Mutex::new(from_kernel),
                to_kernel,
                inject_gate: Semaphore::new(0),
                attempted: tokio::sync::Notify::new(),
                failed: false.into(),
            }),
            send,
            receive,
        )
    }
    pub fn allow_writes(&self) {
        self.inject_gate.add_permits(1);
    }
}

impl tun::Device for MockDevice {
    fn recv<'a>(&'a self, bytes: &'a mut [u8]) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            let data = self
                .from_kernel
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| io::Error::other("mock TUN closed"))?;
            if self.failed.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(io::Error::other("mock device failure"));
            }
            bytes[..data.len()].copy_from_slice(&data);
            Ok(data.len())
        })
    }
    fn send<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            self.attempted.notify_one();
            let _permit = self
                .inject_gate
                .acquire()
                .await
                .map_err(|_| io::Error::other("closed mock gate"))?;
            self.to_kernel
                .send(bytes.to_vec())
                .await
                .map_err(|_| io::Error::other("mock TUN closed"))?;
            Ok(bytes.len())
        })
    }
    fn drain(&self) -> io::Result<()> {
        let mut receive = self
            .from_kernel
            .try_lock()
            .map_err(|_| io::Error::other("concurrent mock generations"))?;
        for _ in 0..256 {
            match receive.try_recv() {
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => return Ok(()),
                Err(_) => return Err(io::Error::other("mock TUN closed")),
            }
        }
        Err(io::Error::other("mock queue exhausted"))
    }
}

#[test]
fn pair_boundary_all_protocols_exact_source_path_and_strict_ipv4() {
    let (payloads, _) = fixtures(3);
    let source = PacketPlan::compile(&payloads[0], 1000).unwrap();
    let target = PacketPlan::compile(&payloads[2], 1000).unwrap();
    let path = &payloads[0].access_policy.grants[0].path;
    for protocol in 0..=255 {
        let bytes = packet("100.64.0.1", "100.64.0.3", protocol);
        assert_eq!(source.outgoing(&bytes), Some(path));
        assert!(target.incoming(&ReceivedPacket {
            source: path[0].clone(),
            path: path.clone(),
            payload: bytes
        }));
    }
    let valid = packet("100.64.0.1", "100.64.0.3", 1);
    let accept = |bytes: Vec<u8>| {
        target.incoming(&ReceivedPacket {
            source: path[0].clone(),
            path: path.clone(),
            payload: bytes,
        })
    };
    for flags in [0x8000_u16, 0x2000, 1, 0x3fff] {
        let mut bytes = valid.clone();
        bytes[6..8].copy_from_slice(&flags.to_be_bytes());
        checksum(&mut bytes);
        assert!(!accept(bytes));
    }
    for (offset, value) in [(0, 0x65), (0, 0x44), (0, 0x46), (8, 0), (10, 0)] {
        let mut bytes = valid.clone();
        bytes[offset] = value;
        assert!(!accept(bytes));
    }
    for length in 0..valid.len() {
        assert!(!accept(valid[..length].to_vec()));
    }
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(!accept(trailing));
    for (from, to) in [
        ("100.64.0.2", "100.64.0.3"),
        ("100.64.0.1", "100.64.0.2"),
        ("100.64.0.1", "8.8.8.8"),
        ("100.64.0.1", "100.64.0.255"),
        ("100.64.0.1", "224.0.0.1"),
    ] {
        let bytes = packet(from, to, 1);
        assert!(!accept(bytes.clone()));
        assert!(source.outgoing(&bytes).is_none());
    }
    assert!(!target.incoming(&ReceivedPacket {
        source: path[1].clone(),
        path: path.clone(),
        payload: valid.clone()
    }));
    assert!(!target.incoming(&ReceivedPacket {
        source: path[0].clone(),
        path: vec![path[0].clone(), path[2].clone()],
        payload: valid
    }));
}

#[test]
fn grants_routes_reverse_and_mtu_are_independent_constraints() {
    let (mut payloads, _) = fixtures(3);
    payloads[0].access_policy.grants.clear();
    assert!(
        PacketPlan::compile(&payloads[0], 1000)
            .unwrap()
            .paths
            .is_empty()
    );
    payloads[1].configuration.routing_policy = RoutingPolicy::default();
    assert!(
        PacketPlan::compile(&payloads[1], 1000)
            .unwrap()
            .paths
            .is_empty()
    );
    payloads[2].access_policy.grants.pop();
    let target = PacketPlan::compile(&payloads[2], 1000).unwrap();
    assert!(
        target
            .outgoing(&packet("100.64.0.3", "100.64.0.1", 1))
            .is_none()
    );
    assert!(PacketPlan::compile(&payloads[2], 1280).is_err());
    let (long, _) = fixtures(8);
    assert!(PacketPlan::compile(&long[0], 1000).is_err());
    assert!(PacketPlan::compile(&long[0], 576).is_ok());
    let plan = PacketPlan::compile(&payloads[2], 1000).unwrap();
    assert!(
        plan.check_targets(&BTreeMap::from([(
            "bad".into(),
            vec!["100.64.0.2:1234".parse().unwrap()]
        )]))
        .is_err()
    );
    assert!(
        plan.check_targets(&BTreeMap::from([(
            "bad".into(),
            vec!["[::ffff:100.64.0.2]:1234".parse().unwrap()]
        )]))
        .is_err()
    );
}
