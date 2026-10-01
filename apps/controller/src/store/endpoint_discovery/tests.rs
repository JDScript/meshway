use super::*;
use crate::NetworkOptions;
use crate::store::test_store::TestStore as Store;

struct Fixture {
    directory: tempfile::TempDir,
    store: Store,
    token: String,
    other: String,
    id: String,
}

impl Fixture {
    fn new() -> Self {
        Self::with_bind("0.0.0.0:19000")
    }
    fn with_bind(bind: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let mut store = Store::open(directory.path(), &NetworkOptions::default()).unwrap();
        let mut enroll = |seed: u8| {
            let enrollment = store.create_token(3600).unwrap();
            let token = local::credential().unwrap();
            let node = store
                .enroll(
                    &enrollment.token,
                    &EnrollAgentRequest {
                        node_credential: token.clone(),
                        name: format!("node-{seed}"),
                        agent_version: "test".into(),
                    },
                )
                .unwrap();
            store
                .publish_connection_identity(
                    &token,
                    &PublishConnectionIdentityRequest {
                        public_key: format!("{seed:02x}{}", "00".repeat(31)),
                        capabilities: vec![ConnectionCapability {
                            transport: ConnectionTransport::NativeUdp,
                            can_dial: true,
                            can_accept: true,
                            semantics: ConnectionSemantics::Datagram,
                        }],
                    },
                )
                .unwrap();
            (node.id, token)
        };
        let (id, token) = enroll(9);
        let (other_id, other) = enroll(10);
        store
            .save_connections(&SaveConnectionsRequest {
                expected_revision: store.revision().unwrap(),
                configuration: ConnectionPlan {
                    listeners: vec![ConnectionListener {
                        id: "listen".into(),
                        node_id: id.clone(),
                        transport: ConnectionTransport::NativeUdp,
                        bind: bind.into(),
                        advertised_endpoints: vec![ConnectionEndpoint {
                            host: "127.0.0.1".into(),
                            port: 19000,
                        }],
                        credential_ref: None,
                    }],
                    candidates: vec![ConnectionCandidate {
                        id: "dial".into(),
                        dialer_node_id: other_id,
                        listener_id: "listen".into(),
                        endpoint: ConnectionEndpoint {
                            host: "127.0.0.1".into(),
                            port: 19000,
                        },
                        priority: 0,
                        credential_ref: None,
                    }],
                },
            })
            .unwrap();
        for credential in [&token, &other] {
            store
                .begin_connection_runtime(
                    credential,
                    &BeginConnectionRuntimeRequest {
                        instance_id: "01".repeat(32),
                        expected_epoch: 0,
                    },
                )
                .unwrap();
            let configuration = store.agent_connections(credential).unwrap();
            store
                .report_connection_runtime(
                    credential,
                    &ReportConnectionRuntimeRequest {
                        epoch: 1,
                        sequence: 1,
                        revision: configuration.payload.revision,
                        digest: configuration.digest,
                        state: ConnectionRuntimeState::Applied,
                        reason: ConnectionRuntimeReason::None,
                        listeners: configuration
                            .payload
                            .listeners
                            .iter()
                            .map(|listener| ConnectionListenerObservation {
                                listener_id: listener.id.clone(),
                                state: ConnectionListenerState::Bound,
                                reason: ConnectionRuntimeReason::None,
                            })
                            .collect(),
                        peers: vec![],
                    },
                )
                .unwrap();
        }
        Self {
            directory,
            store,
            token,
            other,
            id,
        }
    }
    fn request(&self, now: DateTime<Utc>) -> ReportEndpointDiscoveryRequest {
        let configuration = self.store.agent_connections(&self.token).unwrap();
        let bound = configuration.payload.listeners[0].bind.clone();
        let ipv4 = bound.parse::<SocketAddr>().unwrap().is_ipv4();
        ReportEndpointDiscoveryRequest {
            epoch: 1,
            sequence: 1,
            revision: configuration.payload.revision,
            digest: configuration.digest,
            state: EndpointDiscoveryState::Observed,
            sockets: vec![UdpSocketDiscovery {
                resource_kind: UdpDiscoveryResourceKind::Listener,
                resource_id: "listen".into(),
                generation: 1,
                socket_id: 1,
                bound_address: bound,
                source: EndpointDiscoverySource::ManagedUdpSocketStun,
                observations: vec![StunEndpointObservation {
                    observer: if ipv4 {
                        "192.0.2.1:3478"
                    } else {
                        "[2001:db8::1]:3478"
                    }
                    .into(),
                    outcome: StunDiscoveryOutcome::Mapped,
                    mapped_endpoint: Some(
                        if ipv4 {
                            "198.51.100.1:19001"
                        } else {
                            "[2001:db8::2]:19001"
                        }
                        .into(),
                    ),
                    rtt_ms: Some(4),
                    observed_at: now,
                    expires_at: now + Duration::seconds(30),
                }],
            }],
        }
    }
    fn send(
        &mut self,
        request: &ReportEndpointDiscoveryRequest,
        now: DateTime<Utc>,
    ) -> Result<ReportEndpointDiscoveryResponse, Error> {
        self.store
            .report_endpoint_discovery_at(&self.token, request, now)
    }
    fn view(&self, now: DateTime<Utc>) -> NodeEndpointDiscovery {
        self.store
            .endpoint_discoveries_at(now)
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.node_id == self.id)
            .unwrap()
    }
}

#[test]
fn retries_expiry_and_monotonic_observations_are_not_renewal() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let mut report = f.request(now);
    let inventory = serde_json::to_value(f.store.connections().unwrap()).unwrap();
    let admin_bytes = std::fs::read(f.directory.path().join("admin.token")).unwrap();
    let database_bytes = std::fs::read(f.directory.path().join("controller.sqlite3")).unwrap();
    let first = f.send(&report, now).unwrap();
    assert!(f.view(now).current && !f.view(now).stale);
    assert_eq!(
        f.send(&report, now + Duration::seconds(31))
            .unwrap()
            .reported_at,
        first.reported_at
    );
    let expired = f.view(now + Duration::seconds(31));
    assert_eq!(expired.state, EndpointDiscoveryState::Expired);
    assert!(expired.sockets.is_empty());
    report.sequence = 2;
    assert!(matches!(
        f.send(&report, now + Duration::seconds(31)),
        Err(Error::Unprocessable(_))
    ));
    report.sockets[0].observations[0].expires_at += Duration::seconds(1);
    assert!(matches!(
        f.send(&report, now + Duration::seconds(1)),
        Err(Error::Unprocessable(_))
    ));
    report.sockets[0].observations[0].expires_at -= Duration::seconds(2);
    assert!(matches!(
        f.send(&report, now + Duration::seconds(1)),
        Err(Error::Conflict)
    ));
    report.sockets[0].observations[0].observed_at += Duration::seconds(1);
    report.sockets[0].observations[0].expires_at = now + Duration::seconds(31);
    f.send(&report, now + Duration::seconds(1)).unwrap();
    let mut changed = report.clone();
    changed.state = EndpointDiscoveryState::Probing;
    assert!(matches!(f.send(&changed, now), Err(Error::Conflict)));
    changed.sequence = 1;
    assert!(matches!(f.send(&changed, now), Err(Error::Conflict)));
    assert_eq!(
        serde_json::to_value(f.store.connections().unwrap()).unwrap(),
        inventory
    );
    assert_eq!(
        std::fs::read(f.directory.path().join("admin.token")).unwrap(),
        admin_bytes
    );
    assert_eq!(
        std::fs::read(f.directory.path().join("controller.sqlite3")).unwrap(),
        database_bytes
    );
    assert_eq!(
        f.store
            .connection
            .pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        9
    );
}

#[test]
fn complete_snapshots_retire_sockets_and_generations_without_rebinding() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let mut report = f.request(now);
    f.send(&report, now).unwrap();
    report.sequence += 1;
    report.sockets[0].bound_address = "0.0.0.0:19002".into();
    assert!(matches!(f.send(&report, now), Err(Error::Conflict)));
    report.sockets[0].bound_address = "0.0.0.0:19000".into();
    report.sockets[0].generation = 2;
    assert!(matches!(f.send(&report, now), Err(Error::Conflict))); // old observation cannot return in new generation
    report.sockets[0].observations[0].observed_at += Duration::seconds(1);
    f.send(&report, now + Duration::seconds(1)).unwrap();
    report.sequence += 1;
    report.sockets[0].generation = 1;
    assert!(matches!(
        f.send(&report, now + Duration::seconds(1)),
        Err(Error::Conflict)
    ));
    report.sockets[0].generation = 2;
    let old_socket = report.sockets[0].clone();
    report.sockets.clear();
    report.state = EndpointDiscoveryState::Withdrawn;
    f.send(&report, now + Duration::seconds(31)).unwrap(); // expired old samples do not impede withdrawal
    assert!(f.view(now + Duration::seconds(31)).sockets.is_empty());
    report.sequence += 1;
    report.state = EndpointDiscoveryState::Probing;
    report.sockets = vec![old_socket];
    report.sockets[0].observations.clear();
    assert!(matches!(
        f.send(&report, now + Duration::seconds(31)),
        Err(Error::Conflict)
    ));
    report.sockets[0].socket_id = 2;
    f.send(&report, now + Duration::seconds(31)).unwrap();
    report.sequence += 1;
    report.sockets[0].socket_id = 4;
    let mut second = report.sockets[0].clone();
    second.socket_id = 3;
    report.sockets.push(second);
    f.send(&report, now + Duration::seconds(31)).unwrap(); // unordered new ordinals
    assert_eq!(f.view(now + Duration::seconds(31)).sockets.len(), 2);
}

#[test]
fn runtime_evidence_config_revocation_restart_and_server_ttl_filter_claims() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let mut report = f.request(now);
    f.send(&report, now).unwrap();
    f.store
        .connection
        .execute(
            "UPDATE connection_runtime_reports SET reported_at=?1",
            [(now - Duration::seconds(61)).to_rfc3339()],
        )
        .unwrap();
    assert!(!f.view(now).current && f.view(now).stale && f.view(now).sockets.is_empty());
    f.store
        .connection
        .execute(
            "UPDATE connection_runtime_reports SET reported_at=?1",
            [now.to_rfc3339()],
        )
        .unwrap();
    report.sequence += 1;
    report.state = EndpointDiscoveryState::Disabled;
    report.sockets.clear();
    f.send(&report, now).unwrap();
    assert_eq!(f.view(now).state, EndpointDiscoveryState::Disabled);
    f.store
        .connection
        .execute(
            "UPDATE connection_runtime_reports SET reported_at=?1",
            [(now + Duration::seconds(59)).to_rfc3339()],
        )
        .unwrap();
    let expired = f.view(now + Duration::seconds(60));
    assert!(!expired.current && expired.stale);
    assert_eq!(expired.state, EndpointDiscoveryState::Expired);
    f.store
        .begin_connection_runtime(
            &f.token,
            &BeginConnectionRuntimeRequest {
                instance_id: "02".repeat(32),
                expected_epoch: 1,
            },
        )
        .unwrap();
    assert!(!f.view(now).current && f.view(now).sockets.is_empty());
    assert!(matches!(f.send(&report, now), Err(Error::Conflict)));
    report.epoch = 2;
    report.sequence = 1;
    f.send(&report, now).unwrap(); // accepts startup ordering without Applied evidence
    assert!(!f.view(now).current);
    let network_id = f.store.network().unwrap().network_id;
    drop(f.store);
    f.store = Store::open(f.directory.path(), &NetworkOptions::default()).unwrap();
    assert_eq!(f.view(now).state, EndpointDiscoveryState::AwaitingReport);
    assert!(f.view(now).reported_at.is_none());
    assert_eq!(f.store.network().unwrap().network_id, network_id);
    f.store.revoke(&f.id).unwrap();
    assert!(matches!(f.send(&report, now), Err(Error::Forbidden)));
    assert_eq!(f.view(now).state, EndpointDiscoveryState::Withdrawn);
}

#[test]
fn configuration_change_invalidates_live_discovery() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let report = f.request(now);
    f.send(&report, now).unwrap();
    f.store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: f.store.revision().unwrap(),
            configuration: ConnectionPlan {
                listeners: vec![],
                candidates: vec![],
            },
        })
        .unwrap();
    let view = f.view(now);
    assert!(!view.current && view.stale && view.sockets.is_empty());
    assert!(matches!(f.send(&report, now), Err(Error::Unprocessable(_))));
}

#[test]
fn revocation_with_retained_report_clears_claims_without_failing_admin_snapshot() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let report = f.request(now);
    f.send(&report, now).unwrap();
    f.store.revoke(&f.id).unwrap();
    let view = f.view(now);
    assert!(!view.current && view.stale && view.sockets.is_empty());
    assert_eq!(view.state, EndpointDiscoveryState::Withdrawn);
    assert!(matches!(f.send(&report, now), Err(Error::Forbidden)));
}

#[test]
fn observer_rotation_is_bounded_and_retired_samples_cannot_return() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let mut report = f.request(now);
    f.send(&report, now).unwrap();
    let old = report.sockets[0].observations[0].clone();
    report.sequence = 2;
    report.state = EndpointDiscoveryState::Probing;
    report.sockets[0].observations.clear();
    f.send(&report, now).unwrap();
    report.sequence = 3;
    report.state = EndpointDiscoveryState::Observed;
    report.sockets[0].observations.push(old);
    assert!(matches!(f.send(&report, now), Err(Error::Conflict)));
    for index in 1..=8 {
        report.sequence = index + 2;
        report.sockets[0].observations[0].observer = format!("192.0.2.{}:3478", index + 1);
        report.sockets[0].observations[0].observed_at = now + Duration::seconds(i64::from(index));
        f.send(&report, now + Duration::seconds(i64::from(index)))
            .unwrap();
        assert_eq!(f.store.discoveries[&f.id].sockets[&1].observations.len(), 1);
    }
}

#[test]
fn strict_scope_states_limits_addresses_and_times() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let original = f.request(now);
    let mut cases = vec![];
    macro_rules! bad {
        ($body:expr) => {{
            let mut report = original.clone();
            ($body)(&mut report);
            cases.push(report);
        }};
    }
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.digest = "a".repeat(64));
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.revision += 1);
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sequence = 0);
    for state in [
        EndpointDiscoveryState::AwaitingReport,
        EndpointDiscoveryState::Expired,
        EndpointDiscoveryState::Disabled,
        EndpointDiscoveryState::Withdrawn,
        EndpointDiscoveryState::Error,
        EndpointDiscoveryState::Probing,
    ] {
        bad!(|r: &mut ReportEndpointDiscoveryRequest| r.state = state);
    }
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].resource_id = "dial".into());
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].resource_kind =
            UdpDiscoveryResourceKind::OutgoingCandidate
    );
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].generation = 0);
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].socket_id = 0);
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets.push(r.sockets[0].clone()));
    bad!(|r: &mut ReportEndpointDiscoveryRequest| {
        let mut socket = r.sockets[0].clone();
        socket.generation = 2;
        socket.socket_id = 2;
        r.sockets.push(socket);
    });
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets = vec![r.sockets[0].clone(); 129]);
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations =
            vec![r.sockets[0].observations[0].clone(); 5]
    );
    bad!(|r: &mut ReportEndpointDiscoveryRequest| {
        let observation = r.sockets[0].observations[0].clone();
        r.sockets[0].observations.push(observation);
    });
    for value in [
        "host:123",
        "127.0.0.1:0",
        "224.0.0.1:123",
        "255.255.255.255:123",
        "[::ffff:127.0.0.1]:123",
        "[fe80::1%0]:123",
    ] {
        bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].bound_address = value.into());
    }
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].observer =
            "[::1]:3478".into()
    );
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].mapped_endpoint =
            Some("0.0.0.0:123".into())
    );
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].mapped_endpoint =
            None
    );
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].rtt_ms = None);
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].rtt_ms = Some(1501));
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].outcome =
            StunDiscoveryOutcome::Timeout
    );
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].observed_at +=
            Duration::seconds(6)
    );
    bad!(
        |r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].expires_at +=
            Duration::seconds(1)
    );
    bad!(|r: &mut ReportEndpointDiscoveryRequest| r.sockets[0].observations[0].expires_at = now);
    for report in cases {
        assert!(
            matches!(f.send(&report, now), Err(Error::Unprocessable(_))),
            "accepted invalid report {report:?}"
        );
    }
    assert!(matches!(
        f.store
            .report_endpoint_discovery_at(&f.other, &original, now),
        Err(Error::Unprocessable(_))
    ));
    let config = f.store.agent_connections(&f.other).unwrap();
    let mut outgoing = original;
    outgoing.revision = config.payload.revision;
    outgoing.digest = config.digest;
    outgoing.sockets[0].resource_kind = UdpDiscoveryResourceKind::OutgoingCandidate;
    outgoing.sockets[0].resource_id = "dial".into();
    f.store
        .report_endpoint_discovery_at(&f.other, &outgoing, now)
        .unwrap();
}

#[test]
fn probing_failures_ipv6_and_individual_expiry_are_truthful() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let mut report = f.request(now);
    let mut failed = report.sockets[0].observations[0].clone();
    failed.observer = "192.0.2.2:3478".into();
    failed.outcome = StunDiscoveryOutcome::Timeout;
    failed.mapped_endpoint = None;
    failed.rtt_ms = None;
    failed.expires_at = now + Duration::seconds(10);
    report.sockets[0].observations.push(failed);
    f.send(&report, now).unwrap();
    assert_eq!(
        f.view(now + Duration::seconds(11)).sockets[0]
            .observations
            .len(),
        1
    );
    report.sequence += 1;
    report.state = EndpointDiscoveryState::Probing;
    report.sockets[0].socket_id = 2;
    report.sockets[0].observations.remove(0);
    f.send(&report, now).unwrap();
    assert_eq!(f.view(now).state, EndpointDiscoveryState::Probing);
    report.sequence += 1;
    report.sockets[0].observations.clear();
    f.send(&report, now).unwrap();
    let mut ipv6_fixture = Fixture::with_bind("[::]:19000");
    let ipv6 = ipv6_fixture.request(now + Duration::seconds(1));
    ipv6_fixture
        .send(&ipv6, now + Duration::seconds(1))
        .unwrap();
    assert_eq!(
        ipv6_fixture.view(now + Duration::seconds(1)).state,
        EndpointDiscoveryState::Observed
    );
}

#[test]
fn listener_bind_scope_rejects_first_report_and_preserves_previous_snapshot() {
    for bind in [
        "0.0.0.0:19000",
        "127.0.0.1:19000",
        "[::]:19000",
        "[2001:db8::9]:19000",
    ] {
        let mut f = Fixture::with_bind(bind);
        let now = Utc::now();
        let valid = f.request(now);
        let original = serde_json::to_value(f.view(now)).unwrap();
        let configured: SocketAddr = bind.parse().unwrap();
        let mut wrong_port = configured;
        wrong_port.set_port(19001);
        let wrong_family = if configured.is_ipv4() {
            "[::]:19000"
        } else {
            "0.0.0.0:19000"
        };
        let wrong_ip = if configured.is_ipv4() {
            "192.0.2.9:19000"
        } else {
            "[2001:db8::10]:19000"
        };
        let addresses = [
            wrong_family.to_owned(),
            wrong_port.to_string(),
            wrong_ip.to_owned(),
        ];
        for bound in &addresses {
            let mut invalid = valid.clone();
            invalid.state = EndpointDiscoveryState::Probing;
            invalid.sockets[0].observations.clear();
            invalid.sockets[0].bound_address = bound.clone();
            assert!(
                matches!(f.send(&invalid, now), Err(Error::Unprocessable(_))),
                "accepted {bound} for configured bind {bind}"
            );
            assert!(!f.store.discoveries.contains_key(&f.id));
            assert_eq!(serde_json::to_value(f.view(now)).unwrap(), original);
        }
        let first = f.send(&valid, now).unwrap();
        let accepted = serde_json::to_value(f.view(now)).unwrap();
        assert!(f.view(now).current && !f.view(now).stale);
        for bound in &addresses {
            let mut invalid = valid.clone();
            invalid.sequence = 2;
            invalid.state = EndpointDiscoveryState::Probing;
            invalid.sockets[0].generation = 2;
            invalid.sockets[0].observations.clear();
            invalid.sockets[0].bound_address = bound.clone();
            assert!(matches!(
                f.send(&invalid, now),
                Err(Error::Unprocessable(_))
            ));
            assert_eq!(serde_json::to_value(f.view(now)).unwrap(), accepted);
        }
        // Rejected first reports and attempted generation replacements consume
        // neither the report sequence nor any binding/generation watermark.
        assert_eq!(f.send(&valid, now).unwrap().reported_at, first.reported_at);
        let mut replacement = valid;
        replacement.sequence = 2;
        replacement.sockets[0].generation = 2;
        replacement.sockets[0].observations[0].observed_at += Duration::seconds(1);
        f.send(&replacement, now + Duration::seconds(1)).unwrap();
    }
}

#[test]
fn outgoing_literal_family_is_checked_but_dns_family_is_not_guessed() {
    let mut f = Fixture::new();
    let now = Utc::now();
    let config = f.store.agent_connections(&f.other).unwrap();
    let mut outgoing = f.request(now);
    outgoing.digest = config.digest;
    outgoing.revision = config.payload.revision;
    outgoing.state = EndpointDiscoveryState::Probing;
    outgoing.sockets[0].observations.clear();
    outgoing.sockets[0].resource_kind = UdpDiscoveryResourceKind::OutgoingCandidate;
    outgoing.sockets[0].resource_id = "dial".into();
    outgoing.sockets[0].bound_address = "[::]:20000".into();
    assert!(matches!(
        f.store
            .report_endpoint_discovery_at(&f.other, &outgoing, now),
        Err(Error::Unprocessable(_))
    ));
    let other_id = config.payload.node_id;
    assert!(!f.store.discoveries.contains_key(&other_id));
    outgoing.sockets[0].bound_address = "0.0.0.0:20000".into();
    f.store
        .report_endpoint_discovery_at(&f.other, &outgoing, now)
        .unwrap();

    let mut plan = f.store.connections().unwrap().configuration;
    plan.listeners[0].advertised_endpoints[0].host = "carrier.example".into();
    plan.candidates[0].endpoint.host = "carrier.example".into();
    f.store
        .save_connections(&SaveConnectionsRequest {
            expected_revision: f.store.revision().unwrap(),
            configuration: plan,
        })
        .unwrap();
    let config = f.store.agent_connections(&f.other).unwrap();
    outgoing.digest = config.digest;
    outgoing.revision = config.payload.revision;
    outgoing.sequence = 2;
    outgoing.sockets[0].generation = 2;
    outgoing.sockets[0].bound_address = "[::]:20000".into();
    f.store
        .report_endpoint_discovery_at(&f.other, &outgoing, now)
        .unwrap();
    outgoing.sequence = 3;
    outgoing.sockets[0].generation = 3;
    outgoing.sockets[0].bound_address = "0.0.0.0:20000".into();
    f.store
        .report_endpoint_discovery_at(&f.other, &outgoing, now)
        .unwrap();
}
