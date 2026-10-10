//! Synthetic contract/negative cases only; these are not Linux run evidence.
use super::*;
use super::{
    installation::{Installation, Installations},
    verification::Current,
};
use meshway_model::{
    Digest, InstanceId, NetmapRef, Sequence, Version,
    observations::{EndpointKind, LinkAcquisition, ProbeChannel},
    state::*,
};
use meshway_protocol::{
    InstanceRequest,
    state::{StateView, VisibleReport},
};
use serde_json::json;

fn id<T: std::str::FromStr>(n: u8) -> T
where
    T::Err: std::fmt::Debug,
{
    format!("00000000-0000-4000-8000-{n:012}").parse().unwrap()
}
fn cfg(serial: u8) -> NetmapRef {
    NetmapRef {
        revision: 6.try_into().unwrap(),
        serial: Version::new(u64::from(serial)).unwrap(),
        digest: [serial; 32].into(),
    }
}
fn keys(n: u8) -> meshway_protocol::NodeKeyMetadata {
    serde_json::from_value(json!({
        "control":{"id":id::<meshway_model::KeyId>(n+30),"generation":1,"publicKey":{"algorithm":"ed25519","bytes":Digest::from_bytes([n;32])}},
        "dataplane":{"id":id::<meshway_model::KeyId>(n+40),"generation":1,"publicKey":{"algorithm":"x25519","bytes":Digest::from_bytes([n+1;32])}}
    })).unwrap()
}
fn status(n: u8) -> meshway_ipc::Status {
    serde_json::from_value(json!({
        "processId":42,"revision":1,"desiredUp":true,"phase":"running",
        "sync":"disconnected","lastSyncAt":null,
        "server":"http://127.0.0.1:8430/","observedAt":"1970-01-01T00:00:20Z",
        "enrollmentRequestId":null,"enrollmentUrl":null,
        "identity":{"controllerId":id::<meshway_model::KeyId>(99),"networkId":id::<meshway_model::NetworkId>(100),"nodeId":id::<NodeId>(n),"keys":keys(n)},
        "authenticated":true,"instance":{"instanceId":id::<InstanceId>(n+10),"generation":1,"expiresAt":"1970-01-01T00:01:00Z"},
        "netmap":{"netmap":cfg(102),"issuedAt":"1970-01-01T00:00:00Z","expiresAt":"1970-01-01T00:05:00Z"},
        "saved":cfg(102),"applied":cfg(102),"dataPlane":"ready","networkMode":"tun","lastError":null,"retryAt":null
    })).unwrap()
}

struct Fixture {
    observations: Observations,
    current: BTreeMap<NodeId, Current>,
    nodes: BTreeMap<Role, NodeId>,
}
impl Fixture {
    fn new() -> Self {
        let mut observations = Observations::default();
        let nodes = BTreeMap::from([(Role::A, id(1)), (Role::R, id(2)), (Role::B, id(3))]);
        let mut current = BTreeMap::new();
        let mut participants = BTreeMap::new();
        for (role, n) in [(Role::A, 1), (Role::R, 2), (Role::B, 3)] {
            let instance = InstanceRequest {
                instance_id: id(n + 10),
                generation: Version::MIN,
            };
            let socket_id = Digest::from_bytes([n; 32]).to_string();
            let candidate = EndpointCandidate {
                address: format!("192.0.2.{n}:35000"),
                origin: EndpointSource::Stun {
                    observer: "192.0.2.100:3478".into(),
                },
                observed_at: "1970-01-01T00:00:01Z".parse().unwrap(),
                expires_at: "1970-01-01T00:00:12Z".parse().unwrap(),
            };
            let reference = SocketReference {
                node_id: id(n),
                instance: instance.clone(),
                netmap: cfg(101),
                socket_id: socket_id.clone(),
                socket_generation: 7.try_into().unwrap(),
                resource: ResourceRef::Listener { key: "tcp".into() },
            };
            let physical = SocketState {
                local_endpoints: None,
                resource: reference.resource.clone(),
                netmap: Some(cfg(101)),
                generation: reference.socket_generation,
                network: SocketNetwork::Udp,
                bound_address: "0.0.0.0:35000".parse().unwrap(),
                endpoints: vec![candidate.clone()],
            };
            participants.insert(
                n,
                UdpTraversalParticipant {
                    candidate: CandidateReference::from_observation(
                        reference,
                        physical.network,
                        physical.bound_address,
                        &candidate,
                    )
                    .unwrap(),
                    address: candidate.address,
                    source: EndpointKind::Stun,
                },
            );
            observations.events.push(Event::Publication {
                instance_id: instance.instance_id,
                sequence: Sequence::MIN,
                applied: Some(cfg(101)),
                sockets: BTreeMap::from([(socket_id.clone(), physical.clone())]),
            });
            observations.events.push(Event::Query {
                request: QueryUdpTraversalRequest {
                    instance,
                    netmap: cfg(101),
                },
                response: QueryUdpTraversalResponse::Current {
                    state_epoch: id(110),
                    as_of_ms: 1000,
                    expires_at_ms: 10000,
                    stun_servers: vec![StunServer {
                        id: "fixture".into(),
                        endpoint: meshway_transport_spec::Endpoint {
                            host: "192.0.2.100".into(),
                            port: 3478.try_into().unwrap(),
                        },
                    }]
                    .try_into()
                    .unwrap(),
                    attempts: vec![].try_into().unwrap(),
                },
            });
            let mut current_physical = physical;
            current_physical.netmap = Some(cfg(102));
            // A later STUN observation is not the original offer's evidence.
            current_physical.endpoints[0].observed_at = "1970-01-01T00:00:19Z".parse().unwrap();
            current_physical.endpoints[0].expires_at = "1970-01-01T00:00:39Z".parse().unwrap();
            let report = json!({ "system":{"hostname":role.name(),"os":"linux","arch":"aarch64","agentVersion":"test"},"capabilities":{},
                "configuration":{"saved":cfg(102),"applied":cfg(102),"progress":{"target":cfg(102),"phase":"applied","observedAt":"1970-01-01T00:00:20Z","problem":null}},
                "listeners":{},"sockets":{socket_id:current_physical},"links":{},"paths":{},"selection":{"appliedProfile":null},"probes":[],"problems":[] });
            let node = id::<NodeId>(n);
            let view: StateView = serde_json::from_value(json!({ "networkId":id::<meshway_model::NetworkId>(100),"stateEpoch":id::<meshway_model::StateEpoch>(110),"sequence":20,"configurationRevision":6,"asOf":"1970-01-01T00:00:20Z",
                "nodes":{node.to_string():{"hostname":role.name(),"instanceId":id::<InstanceId>(n+10),"reportSequence":20,"observedAt":"1970-01-01T00:00:20Z","receivedAt":"1970-01-01T00:00:20Z","reportExpiresAt":"1970-01-01T00:01:00Z","lastAuthenticatedAt":"1970-01-01T00:00:20Z","freshness":"current","report":{"view":"full","report":report},"derived":{"ipInfo":{},"location":null,"locationAbsent":null}}},"links":[],"paths":[] })).unwrap();
            current.insert(
                node,
                Current {
                    view,
                    installations: Installations {
                        role,
                        status: Box::new(status(n)),
                        installed: Vec::new(),
                    },
                },
            );
        }
        for (local, peer) in [(1, 2), (2, 3)] {
            let mut offer = UdpTraversalOffer {
                attempt_id: id(local + 60),
                request_id: id(local + 70),
                network_id: id(100),
                state_epoch: id(110),
                version: Version::MIN,
                digest: [0; 32].into(),
                nonce: [local + 90; 32].into(),
                carrier: meshway_configuration::execution::Carrier {
                    dialer: id(local),
                    listener: id(peer),
                    listener_key: "tcp".into(),
                    protocol: meshway_model::TransportProtocol::NativeUdp,
                },
                local: participants[&local].clone(),
                peer: participants[&peer].clone(),
                expires_at_ms: 10000,
            };
            offer.digest = offer.compute_digest().unwrap();
            observations.events.push(Event::Prepare {
                request: Box::new(PrepareUdpTraversalRequest {
                    instance: offer.local.candidate.socket.instance.clone(),
                    netmap: cfg(101),
                    state_epoch: offer.state_epoch,
                    request_id: offer.request_id,
                    expires_at_ms: 10000,
                    peer_id: id(peer),
                    carrier: offer.carrier.clone(),
                    local_candidate: offer.local.candidate.clone(),
                    peer_candidate: offer.peer.candidate.clone(),
                }),
                response: PrepareUdpTraversalResponse::Offer {
                    offer: offer.clone(),
                },
            });
            for (n, other) in [(local, peer), (peer, local)] {
                observations.events.push(Event::Accept {
                    request: Box::new(UdpTraversalAttemptRequest {
                        instance: participants[&n].candidate.socket.instance.clone(),
                        netmap: cfg(101),
                        state_epoch: offer.state_epoch,
                        version: offer.version,
                        digest: offer.digest,
                    }),
                    response: if n == local {
                        AcceptUdpTraversalResponse::Waiting {
                            offer: offer.clone(),
                        }
                    } else {
                        AcceptUdpTraversalResponse::Active {
                            offer: offer.clone(),
                        }
                    },
                    received_at_ms: 2000,
                });
                let node = id(n);
                let keys = keys(n);
                let other_keys = self::keys(other);
                let installation = Installation {
                    node,
                    network: offer.network_id,
                    instance: participants[&n].candidate.socket.instance.clone(),
                    netmap: cfg(101),
                    observed_at_ms: 3000,
                    control_key: keys.control,
                    dataplane_key: keys.dataplane,
                    peer_control_key: other_keys.control,
                    peer_dataplane_key: other_keys.dataplane,
                    carrier: offer.carrier.clone(),
                    channel: ProbeChannel {
                        link_id: [local + 80; 32].into(),
                        socket_id: [n; 32].into(),
                        peer_node_id: id(other),
                        transport: offer.carrier.protocol,
                        local_address: "0.0.0.0:35000".parse().unwrap(),
                        remote_address: format!("192.0.2.{other}:35000").parse().unwrap(),
                        acquisition: LinkAcquisition::CoordinatedUdp {
                            attempt_id: offer.attempt_id,
                            endpoint_source: EndpointKind::Stun,
                        },
                    },
                };
                let context = current.get_mut(&node).unwrap();
                let VisibleReport::Full { report } = context
                    .view
                    .nodes
                    .get_mut(&node)
                    .unwrap()
                    .report
                    .as_mut()
                    .unwrap()
                else {
                    panic!()
                };
                report.links.insert(
                    id(other),
                    vec![LinkState {
                        acquisition: None,
                        link_id: installation.channel.link_id.to_string(),
                        socket_id: installation.channel.socket_id.to_string(),
                        transport: offer.carrier.protocol.as_str().into(),
                        initiator_node_id: offer.carrier.dialer,
                        phase: LinkPhase::Authenticated,
                        business_dependencies: 1,
                        observation_ids: vec![],
                        observed_at: "1970-01-01T00:00:20Z".parse().unwrap(),
                        expires_at: "1970-01-01T00:01:00Z".parse().unwrap(),
                        since: None,
                    }],
                );
                context.installations.installed.push(installation);
            }
        }
        Self {
            observations,
            current,
            nodes,
        }
    }
    fn verify(&self) -> bool {
        verification::verify(
            &self.observations,
            &self.current,
            "192.0.2.100:3478",
            &self.nodes,
        )
        .is_ok()
    }
    fn report(&mut self) -> &mut NodeState {
        let VisibleReport::Full { report } = self
            .current
            .get_mut(&id(1))
            .unwrap()
            .view
            .nodes
            .get_mut(&id(1))
            .unwrap()
            .report
            .as_mut()
            .unwrap()
        else {
            panic!()
        };
        report
    }
    fn installed(&mut self) -> &mut Installation {
        &mut self
            .current
            .get_mut(&id(1))
            .unwrap()
            .installations
            .installed[0]
    }
}

#[test]
fn publication_handoff_keeps_original_offer_and_installation_but_requires_current_execution() {
    let fixture = Fixture::new();
    assert!(fixture.verify());
    let (_, proofs) = verification::verify(
        &fixture.observations,
        &fixture.current,
        "192.0.2.100:3478",
        &fixture.nodes,
    )
    .unwrap();
    assert_eq!(proofs.len(), 4);
    let encoded = serde_json::to_value(proofs).unwrap();
    assert_ne!(
        encoded[0]["installation"]["netmap"],
        encoded[0]["currentNetmap"]
    );
}

#[test]
fn changed_channel_socket_owner_scope_keys_or_tuple_cannot_borrow_an_old_offer() {
    for mutation in 0..20 {
        let mut f = Fixture::new();
        match mutation {
            0 => f.report().sockets.values_mut().next().unwrap().generation = 8.try_into().unwrap(),
            1 => {
                f.report().sockets.values_mut().next().unwrap().resource = ResourceRef::Listener {
                    key: "replaced".into(),
                }
            }
            2 => f.report().sockets.values_mut().next().unwrap().netmap = Some(cfg(103)),
            3 => {
                f.report().links.values_mut().next().unwrap()[0].link_id =
                    Digest::from_bytes([9; 32]).to_string()
            }
            4 => {
                f.report().links.values_mut().next().unwrap()[0].socket_id =
                    Digest::from_bytes([9; 32]).to_string()
            }
            5 => {
                f.installed().channel.acquisition = LinkAcquisition::CoordinatedUdp {
                    attempt_id: id(98),
                    endpoint_source: EndpointKind::Stun,
                }
            }
            6 => f.installed().netmap = cfg(103),
            7 => f.installed().instance.instance_id = id(98),
            8 => f.installed().channel.remote_address = "192.0.2.99:35000".parse().unwrap(),
            9 => f.installed().control_key.public_key.bytes = [9; 32].into(),
            10 => f.installed().peer_dataplane_key.generation = 2.try_into().unwrap(),
            11 => {
                f.current
                    .get_mut(&id(1))
                    .unwrap()
                    .view
                    .nodes
                    .get_mut(&id(1))
                    .unwrap()
                    .instance_id = Some(id(98))
            }
            12 => f.current.get_mut(&id(1)).unwrap().view.state_epoch = id(98),
            13 => f.installed().node = id(98),
            14 => f.installed().observed_at_ms = 10001,
            15 => f.report().configuration.applied = None,
            16 => f.report().links.values_mut().next().unwrap()[0].business_dependencies = 0,
            17 => f.report().links.values_mut().next().unwrap()[0].phase = LinkPhase::Closed,
            18 => f.installed().channel.socket_id = [9; 32].into(),
            19 => {
                f.current
                    .get_mut(&id(1))
                    .unwrap()
                    .installations
                    .status
                    .netmap
                    .as_mut()
                    .unwrap()
                    .netmap = cfg(103)
            }
            _ => unreachable!(),
        }
        assert!(!f.verify(), "mutation {mutation} admitted");
    }
}

#[test]
fn original_offer_request_candidate_and_acceptance_remain_exact() {
    for mutation in 0..8 {
        let mut f = Fixture::new();
        for event in &mut f.observations.events {
            match event {
                Event::Prepare {
                    request,
                    response: PrepareUdpTraversalResponse::Offer { offer },
                } if offer.local.candidate.socket.node_id == id(1) => match mutation {
                    0 => offer.nonce = [9; 32].into(),
                    1 => request.netmap = cfg(103),
                    2 => request.state_epoch = id(98),
                    _ => {}
                },
                Event::Accept {
                    request,
                    received_at_ms,
                    ..
                } if request.instance.instance_id == id(11) => match mutation {
                    3 => request.digest = [9; 32].into(),
                    4 => request.netmap = cfg(103),
                    5 => *received_at_ms = 10000,
                    _ => {}
                },
                Event::Publication {
                    instance_id,
                    sockets,
                    ..
                } if *instance_id == id(11) => match mutation {
                    6 => {
                        sockets.values_mut().next().unwrap().endpoints[0].origin =
                            EndpointSource::Stun {
                                observer: "wrong:3478".into(),
                            }
                    }
                    7 => {
                        sockets.values_mut().next().unwrap().endpoints[0].expires_at =
                            "1970-01-01T00:00:09Z".parse().unwrap()
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        assert!(!f.verify(), "mutation {mutation} admitted");
    }
}

#[test]
fn installations_roundtrip_through_the_actual_tagged_signal_decoder() {
    let mut fixture = Fixture::new();
    let mut evidence = fixture.current.remove(&id(1)).unwrap().installations;
    evidence.installed[0].observed_at_ms = u64::MAX;
    let signal = crate::signals::Signal::Installations {
        evidence: Box::new(evidence),
    };
    let bytes = serde_json::to_vec(&signal).unwrap();
    let decoded = crate::signals::Signal::decode("traversal-installations-a", &bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
}

#[test]
fn legacy_debug_install_log_is_not_provenance() {
    let event = json!({"target":"meshway_connectivity::manager","fields":{"message":"Connectivity channel transition","operation":"install","cause":"authenticated_ready","attempt":"None","channel":"[1, 2, 3]"}});
    assert!(installation::decode(&event).is_err());
}

#[test]
fn typed_install_fields_decode_without_interpreting_legacy_debug_fields() {
    let mut fixture = Fixture::new();
    let expected = fixture.installed().clone();
    let fields = json!({
        "message":"Connectivity channel transition","operation":"install","cause":"authenticated_ready",
        "attempt":"None","channel":"not a DTO","socket":"not a DTO","protocol":"not a DTO",
        "install_schema":3,"install_json":serde_json::to_string(&expected).unwrap()
    });
    let mut event = json!({"target":"meshway_connectivity::manager","fields":fields});
    let decoded = installation::decode(&event).unwrap().unwrap();
    assert_eq!(
        serde_json::to_vec(&decoded).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
    let mut invalid = serde_json::to_value(&expected).unwrap();
    invalid["channel"]["acquisition"]["attemptId"] = json!("None");
    event["fields"]["install_json"] = json!(serde_json::to_string(&invalid).unwrap());
    assert!(installation::decode(&event).is_err());
    event["fields"]["install_json"] = json!(serde_json::to_string(&expected).unwrap());
    event["fields"]["install_schema"] = json!(2);
    assert!(installation::decode(&event).is_err());
}
