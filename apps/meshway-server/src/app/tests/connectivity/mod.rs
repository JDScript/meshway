//! Registered identities and real compiled publications. Reported documentation
//! addresses exercise the approved self-report contract, not packet reachability.
use super::{
    App,
    node::{caps, fixture, session},
};
use crate::{accounts::random_uuid, state::service::ReadIdentity};
use meshway_configuration::execution::{
    BusinessProtocol, BusinessTarget, Carrier, CompiledNode, DemandDecision,
};
use meshway_model::{
    ConfigurationRef, InstanceId, NetworkId, NodeId, RequestId, Sequence, StateEpoch, Version,
    state::*,
};
use meshway_protocol::{
    connectivity::*,
    execution::*,
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
    target: CompiledNode,
    report: StateReport,
}
impl Node {
    fn configuration(&self) -> ConfigurationRef {
        ConfigurationRef {
            revision: self.target.configuration.revision,
            execution_publication: self.target.configuration.publication,
            digest: self.target.digest,
        }
    }
    fn query(&self) -> QueryUdpTraversalRequest {
        QueryUdpTraversalRequest {
            instance: self.instance.clone(),
            configuration: self.configuration(),
        }
    }
    fn candidate(&self) -> CandidateReference {
        let socket = &self.report.state.sockets["socket"];
        CandidateReference::from_observation(
            SocketReference {
                node_id: self.id,
                instance: self.instance.clone(),
                configuration: self.configuration(),
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
            configuration: self.configuration(),
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

fn report(target: &CompiledNode, instance: &InstanceRequest, address: &str) -> StateReport {
    let now = chrono::Utc::now();
    let reference = ConfigurationRef {
        revision: target.configuration.revision,
        execution_publication: target.configuration.publication,
        digest: target.digest,
    };
    StateReport {
        instance_id: instance.instance_id,
        sequence: Sequence::MIN,
        observed_at: now,
        state: NodeState {
            system: SystemState {
                hostname: "reported-fact".into(),
                os: "linux".into(),
                arch: "aarch64".into(),
                agent_version: "test".into(),
            },
            capabilities: target.configuration.self_keys.capabilities.clone(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference.clone()),
                progress: None,
            },
            listeners: BTreeMap::from([(
                "udp".into(),
                ListenerState {
                    phase: ListenerPhase::Listening,
                    socket_ids: vec!["socket".into()],
                    loaded_certificate_sha256: None,
                    problem: None,
                    configuration: Some(reference.clone()),
                },
            )]),
            sockets: BTreeMap::from([(
                "socket".into(),
                SocketState {
                    resource: ResourceRef::Listener { key: "udp".into() },
                    configuration: Some(reference),
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
            paths: BTreeMap::new(),
            selection: SelectionState {
                applied_profile: Some(target.configuration.dns.profile.clone()),
            },
            probes: vec![],
            problems: vec![],
        },
    }
}

async fn sync_execution(app: &App, token: &str, instance: &InstanceRequest) -> CompiledNode {
    let response = app
        .execution()
        .refresh_configuration(
            token,
            NodeConfigurationRequest {
                instance: instance.clone(),
                base: None,
            },
        )
        .await
        .unwrap();
    let target = response.full.unwrap();
    for (sequence, phase) in [(1, ReceiptPhase::Saved), (2, ReceiptPhase::Applied)] {
        app.execution()
            .receipt(
                token,
                ReceiptRequest {
                    instance: instance.clone(),
                    target: response.lease.target.clone().unwrap(),
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
    target
}

async fn sync(
    app: &App,
    id: NodeId,
    token: String,
    instance: InstanceRequest,
    address: &str,
) -> Node {
    let target = sync_execution(app, &token, &instance).await;
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
    let mut report = report(&target, &instance, address);
    report.sequence = previous
        .nodes
        .get(&id)
        .and_then(|node| node.report_sequence)
        .unwrap_or(Sequence::MIN);
    let mut node = Node {
        id,
        token,
        instance,
        target,
        report,
    };
    node.publish(app).await;
    node
}

async fn setup(business: bool) -> (tempfile::TempDir, App, NetworkId, Node, Node, RequestId) {
    let (dir, app, network, a, b, ak, bk, signer_a, signer_b) = fixture().await;
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
    let demand = rid();
    if business {
        let response = app
            .execution()
            .demand(
                at.token.expose(),
                DemandRequest {
                    instance: instances[0].clone(),
                    request_id: demand,
                    target: BusinessTarget::Mesh { node: b },
                    protocol: BusinessProtocol::Udp,
                    port: Some(53.try_into().unwrap()),
                    profile: "mesh".into(),
                    duration_seconds: 60,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            response.decision,
            DemandDecision::Permitted { .. }
        ));
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
    (dir, app, network, a, b, demand)
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
        configuration: a.configuration(),
        state_epoch,
        request_id: rid(),
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 9_000,
        peer_id: b.id,
        carrier: Carrier {
            dialer: a.id,
            listener: b.id,
            listener_key: "udp".into(),
            protocol: meshway_transport_spec::TransportProtocol::NativeUdp,
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

mod transactions;

mod visibility;
