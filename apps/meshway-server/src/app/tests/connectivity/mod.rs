//! UDP coordination on the netmap model: registered identities, real
//! signed netmaps and State reports. Reported documentation addresses
//! exercise the self-report contract, not packet reachability.
use super::{
    App,
    node::{caps, fixture, session},
};
use crate::{accounts::random_uuid, state::service::ReadIdentity};
use meshway_configuration::{TopologyDefault, execution::Carrier, netmap::CompiledNetmap};
use meshway_model::{
    InstanceId, NetmapRef, NetworkId, NodeId, RequestId, Sequence, StateEpoch, Version, state::*,
};
use meshway_protocol::{
    connectivity::*,
    execution::*,
    netmap::{SyncNetmap, SyncRequest},
    state::{StateReport, VisibleReport},
};
use std::collections::BTreeMap;

fn rid() -> RequestId {
    RequestId::new(random_uuid().unwrap()).unwrap()
}

struct Node {
    id: NodeId,
    token: String,
    instance: InstanceRequest,
    netmap: CompiledNetmap,
    report: StateReport,
}
impl Node {
    fn reference(&self) -> NetmapRef {
        self.netmap.reference()
    }
    fn query(&self) -> QueryUdpTraversalRequest {
        QueryUdpTraversalRequest {
            instance: self.instance.clone(),
            netmap: self.reference(),
        }
    }
    fn candidate(&self) -> CandidateReference {
        let socket = &self.report.state.sockets["socket"];
        CandidateReference::from_observation(
            SocketReference {
                node_id: self.id,
                instance: self.instance.clone(),
                netmap: self.reference(),
                socket_id: "socket".into(),
                socket_generation: socket.generation,
                resource: socket.resource.clone(),
            },
            socket.network,
            socket.bound_address,
            &socket.endpoints[0],
        )
        .unwrap()
    }
    fn cas(&self, offer: &UdpTraversalOffer) -> UdpTraversalAttemptRequest {
        UdpTraversalAttemptRequest {
            instance: self.instance.clone(),
            netmap: self.reference(),
            state_epoch: offer.state_epoch,
            version: offer.version,
            digest: offer.digest,
        }
    }
    async fn publish(&mut self, app: &App) {
        self.report.sequence = self.report.sequence.checked_next().unwrap();
        app.node_state_service()
            .unwrap()
            .accept_report(self.token.clone(), self.report.clone())
            .await
            .unwrap();
    }
}

fn report(netmap: &CompiledNetmap, instance: &InstanceRequest, address: &str) -> StateReport {
    let now = chrono::Utc::now();
    let reference = netmap.reference();
    StateReport {
        instance_id: instance.instance_id,
        sequence: Sequence::MIN,
        observed_at: now,
        state: NodeState {
            exit_capability: None,
            exit_dns: None,
            system: SystemState {
                hostname: "reported-fact".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test".into(),
            },
            capabilities: caps(),
            configuration: ConfigurationState {
                saved: Some(reference),
                applied: Some(reference),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["socket".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    netmap: Some(reference),
                },
            )]),
            sockets: BTreeMap::from([(
                "socket".into(),
                SocketState {
                    local_endpoints: None,
                    resource: ResourceRef::Listener { key: "udp".into() },
                    netmap: Some(reference),
                    generation: Sequence::new(1).unwrap(),
                    network: SocketNetwork::Udp,
                    bound_address: "0.0.0.0:23456".parse().unwrap(),
                    endpoints: vec![EndpointCandidate {
                        address: address.into(),
                        origin: EndpointSource::Stun {
                            observer: "203.0.113.53:3478".into(),
                        },
                        observed_at: now,
                        expires_at: now + chrono::Duration::seconds(30),
                    }],
                },
            )]),
            links: BTreeMap::new(),
            adjacencies: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: SelectionState {
                applied_profile: Some(netmap.netmap.dns.profile.clone()),
            },
            probes: vec![],
            problems: vec![],
        },
    }
}

async fn sync_netmap(app: &App, token: &str, instance: &InstanceRequest) -> CompiledNetmap {
    let response = app
        .netmaps()
        .sync(
            token,
            SyncRequest {
                netmap: None,
                state: None,
                server_key_generation: None,
                traversal_requests: Default::default(),
                associations: None,
            },
        )
        .await
        .unwrap();
    let SyncNetmap::Full { netmap } = response.netmap else {
        panic!("full netmap")
    };
    for (sequence, phase) in [(1, ReceiptPhase::Saved), (2, ReceiptPhase::Applied)] {
        app.execution()
            .receipt(
                token,
                ReceiptRequest {
                    instance: instance.clone(),
                    netmap: netmap.reference(),
                    phase,
                    attempt: Version::MIN,
                    sequence: Sequence::new(sequence).unwrap(),
                    reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
                    failure: None,
                },
            )
            .await
            .unwrap();
    }
    *netmap
}

async fn sync(
    app: &App,
    id: NodeId,
    token: String,
    instance: InstanceRequest,
    address: &str,
) -> Node {
    let netmap = sync_netmap(app, &token, &instance).await;
    let previous = app
        .node_state_service()
        .unwrap()
        .query(
            ReadIdentity::Node {
                authorization: token.clone(),
            },
            None,
        )
        .await
        .unwrap();
    let mut report = report(&netmap, &instance, address);
    report.sequence = previous
        .nodes
        .get(&id)
        .and_then(|node| node.report_sequence)
        .unwrap_or(Sequence::MIN);
    let mut node = Node {
        id,
        token,
        instance,
        netmap,
        report,
    };
    node.publish(app).await;
    node
}

/// Owner session of the fixture network.
async fn owner(app: &App) -> String {
    app.accounts()
        .login(
            "owner@example.com".into(),
            crate::accounts::Secret::new("a sufficiently long password".into()),
            rid(),
        )
        .await
        .unwrap()
        .token
        .expose()
        .to_owned()
}

/// Remove every adjacency (topology deny, no rules): no carrier admits a
/// traversal between the two Nodes any more.
async fn withdraw_adjacency(app: &App, network: NetworkId) {
    let owner = owner(app).await;
    let current = app
        .configurations()
        .read(&owner, network, None)
        .await
        .unwrap();
    app.configurations()
        .apply(
            &owner,
            network,
            meshway_protocol::configuration::ConfigurationRequest::Operations {
                expected_revision: current.revision,
                request_id: rid(),
                ops: vec![
                    meshway_configuration::ConfigurationOperation::ReplaceTopology {
                        topology: vec![],
                    },
                    meshway_configuration::ConfigurationOperation::ReplaceDefaults {
                        defaults: serde_json::from_value(serde_json::json!({ "topology": "deny" }))
                            .unwrap(),
                    },
                ],
            },
            rid(),
        )
        .await
        .unwrap();
    let _ = TopologyDefault::Deny;
}

/// Two registered Nodes with installed netmaps and published UDP candidates.
/// Without `adjacent`, their topology admits no carrier between them.
async fn setup(adjacent: bool) -> (tempfile::TempDir, App, NetworkId, Node, Node) {
    let (dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
    if !adjacent {
        withdraw_adjacency(&app, network).await;
    }
    let at = session(&app.execution(), network, a, &ak, &signer_a).await;
    let bt = session(&app.execution(), network, b, &bk, &signer_b).await;
    let mut instances = Vec::new();
    for token in [at.token.expose(), bt.token.expose()] {
        let lease = app
            .execution()
            .acquire(
                token,
                AcquireInstanceRequest {
                    instance_id: InstanceId::new(random_uuid().unwrap()).unwrap(),
                    capabilities: caps(),
                },
            )
            .await
            .unwrap();
        instances.push(InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        });
    }
    let a = sync(
        &app,
        a,
        at.token.expose().into(),
        instances[0].clone(),
        "198.51.100.1:23456",
    )
    .await;
    let b = sync(
        &app,
        b,
        bt.token.expose().into(),
        instances[1].clone(),
        "198.51.100.2:23456",
    )
    .await;
    (dir, app, network, a, b)
}

async fn request(app: &App, a: &Node, b: &Node) -> PrepareUdpTraversalRequest {
    let QueryUdpTraversalResponse::Current { state_epoch, .. } =
        app.connectivity().query(&a.token, a.query()).await.unwrap()
    else {
        panic!("current metadata");
    };
    request_in_epoch(a, b, state_epoch)
}

fn request_in_epoch(a: &Node, b: &Node, state_epoch: StateEpoch) -> PrepareUdpTraversalRequest {
    PrepareUdpTraversalRequest {
        instance: a.instance.clone(),
        netmap: a.reference(),
        state_epoch,
        request_id: rid(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 9_000,
        peer_id: b.id,
        carrier: Carrier {
            dialer: a.id,
            listener: b.id,
            listener_key: "udp".into(),
            protocol: meshway_model::TransportProtocol::NativeUdp,
        },
        local_candidate: a.candidate(),
        peer_candidate: b.candidate(),
    }
}

async fn offer(app: &App, a: &Node, request: PrepareUdpTraversalRequest) -> UdpTraversalOffer {
    let result = app.connectivity().prepare(&a.token, request).await.unwrap();
    let PrepareUdpTraversalResponse::Offer { offer } = result else {
        panic!("expected offer, got {result:?}");
    };
    assert_eq!(offer.compute_digest().unwrap(), offer.digest);
    offer
}

mod authority;
mod expiry;
mod signalling;
mod transactions;
