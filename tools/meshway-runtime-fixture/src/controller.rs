use crate::{
    Cli, DEADLINE, Error, IO, Result, STARTUP, request,
    signals::{self, Addresses, Role, Signal, Signals},
};
use meshway_configuration::{
    AccessRule, Input, MeshRoute, NetworkConfigurationInput, TopologyAction, TopologyDefault,
    TopologyRule,
};
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery, GetDeviceAuthorizationPath, GetNodeStatePath, ListNodesPath,
};
use meshway_model::{NetworkId, NodeId, Revision};
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, RegisterRequest,
    WireSecret,
    state::{Freshness, VisibleReport},
};
use meshway_server::{App, Settings};
use meshway_transport_spec::{
    DialerSet, Endpoint, Listener, NativeTcpDialer, NativeTcpListener, NativeUdpDialer,
    NativeUdpListener,
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

pub async fn run(cli: &Cli) -> Result<()> {
    let dns = crate::dns::Upstream::start(cli).await?;
    let result = run_server(cli).await;
    let dns_result = dns.shutdown(cli).await;
    result.and(dns_result)
}

async fn run_server(cli: &Cli) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8443").await?;
    let tls = meshway_packet_fixture::tls_server(&cli.public, &cli.private).await?;
    let mut settings = Settings::new(cli.state.join("server"));
    let audit = crate::traversal::Audit::default();
    if cli.traversal == crate::TraversalMode::Nat {
        settings.stun_listen = vec!["0.0.0.0:3478".parse()?];
        settings.stun_servers = BTreeMap::from([(
            "fixture".into(),
            Endpoint {
                host: signals::underlay(cli).await?.controller.to_string(),
                port: 3478.try_into()?,
            },
        )]);
    }
    let app = App::open(settings).await?;
    let mut router = meshway_server::http::router(&app);
    if cli.traversal == crate::TraversalMode::Nat {
        router = router.layer(axum::middleware::from_fn_with_state(
            audit.clone(),
            crate::traversal::observe,
        ));
    }
    let (stop, mut stopped) = watch::channel(false);
    let server = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result??; }
                    accepted = listener.accept(), if connections.len() < 64 => {
                        let (stream, _) = accepted?;
                        let tls = tls.clone();
                        let router = router.clone();
                        connections.spawn(async move {
                            let Ok(stream) = tls.accept(stream).await else {
                                tracing::debug!(phase = "client-connection-closed", stage = "tls", "Client connection ended before TLS completed");
                                return Ok::<(), Error>(());
                            };
                            let service = hyper_util::service::TowerToHyperService::new(router);
                            if let Err(error) = hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(stream), service).await
                            {
                                // A daemon restart may close a keepalive stream
                                // without TLS close_notify. One client's I/O or
                                // protocol failure does not stop the listener;
                                // coordinate() still verifies every API outcome.
                                tracing::debug!(phase = "client-connection-closed", stage = "http",
                                    incomplete = error.is_incomplete_message(), closed = error.is_closed(),
                                    "Client HTTP connection ended");
                            }
                            Ok::<(), Error>(())
                        });
                    }
                }
            }
            Ok(())
        }.await;
        connections.abort_all();
        let mut cleanup = Ok(());
        while let Some(joined) = connections.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(error)) => cleanup = Err(error),
                Err(error) if error.is_cancelled() => {}
                Err(error) => cleanup = Err(error.into()),
            }
        }
        result.and(cleanup)
    });
    tracing::info!(phase = "controller-listening", "real Server HTTPS ready");
    let result = tokio::time::timeout(DEADLINE, coordinate(cli, &audit)).await;
    let audit_result = audit.save(cli).await;
    // Always shut down the actual listener, SSE streams, and database owner,
    // including when an enrollment/configuration/traffic assertion fails.
    let stop_result = stop
        .send(true)
        .map_err(|_| "controller stop channel closed");
    app.stream_shutdown()();
    let server_result = server.await;
    let store_result = app.shutdown().await;
    stop_result?;
    server_result??;
    store_result?;
    audit_result?;
    result??;
    let signals = Signals::new(&cli.signals);
    signals.write("offline", &Signal::Ready).await?;
    tracing::info!(
        phase = "controller-offline",
        "actual listener and Server owner stopped"
    );
    for role in Role::ALL {
        let report = signals
            .wait_for(&format!("report-{}", role.name()), Duration::from_secs(90))
            .await?;
        match &report {
            Signal::Report {
                role: claimed,
                udp4,
                udp6,
                tcp4,
                tcp6,
                relay_tun_received,
                tun_removed: true,
                capture_removed: true,
                identity_preserved: true,
                dns,
                fallback,
                ..
            } if *claimed == role
                && match fallback {
                    Some(proof) => {
                        cli.traversal_fallback
                            && role == Role::A
                            && proof.passed
                            && proof.tcp_circuits.len() == 2
                    }
                    None => !(cli.traversal_fallback && role == Role::A),
                }
                && (dns.as_ref().is_some_and(|evidence| {
                    evidence.backend == cli.dns_backend
                        && evidence.original_resolv_conf_restored == Some(true)
                        && evidence.original_links_restored == Some(true)
                }) == cli.dns_backend.enabled())
                && match role {
                    Role::A | Role::B => *udp4 > 0 && *udp6 > 0 && *tcp4 > 0 && *tcp6 > 0,
                    Role::R => {
                        *relay_tun_received == Some(0)
                            && *udp4 == 0
                            && *udp6 == 0
                            && *tcp4 == 0
                            && *tcp6 == 0
                    }
                } => {}
            _ => return Err("incomplete runtime actor evidence".into()),
        }
        tokio::fs::write(
            cli.evidence.join(format!("report-{}.json", role.name())),
            serde_json::to_vec_pretty(&report)?,
        )
        .await?;
    }
    Ok(())
}

async fn source(
    client: &Client,
    network: NetworkId,
    token: &str,
) -> Result<meshway_protocol::ConfigurationSourceResponse> {
    Ok(client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: network,
            },
            &GetConfigurationSourceQuery::default(),
            token,
        )
        .await?)
}

async fn apply(
    client: &Client,
    network: NetworkId,
    token: &str,
    revision: Revision,
    input: &NetworkConfigurationInput,
) -> Result<Revision> {
    Ok(client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: network,
            },
            &ConfigurationRequest::Document {
                expected_revision: revision,
                request_id: request()?,
                source_jsonc: WireSecret::new(serde_json::to_string(input)?),
            },
            token,
        )
        .await?
        .revision)
}

async fn coordinate(cli: &Cli, audit: &crate::traversal::Audit) -> Result<()> {
    let signals = Signals::new(&cli.signals);
    let underlay = signals::underlay(cli).await?;
    let client = Client::new(
        meshway_packet_fixture::CONTROLLER_ORIGIN,
        meshway_packet_fixture::tls_client(&cli.public, IO)
            .await?
            .resolve(
                meshway_packet_fixture::CONTROLLER_NAME,
                SocketAddr::new(underlay.controller.into(), 8443),
            ),
        16 * 1024 * 1024,
    )?;
    let session = client
        .register(&RegisterRequest {
            email: "isolated-runtime-owner@example.test".into(),
            password: WireSecret::new(format!("runtime-owner-{}", request()?)),
        })
        .await?;
    let token = session.token.expose();
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: request()?,
                name: "Real Linux daemon conformance".into(),
            },
            token,
        )
        .await?
        .network
        .id;
    // Commit the supported network policy before approving any daemon. In
    // particular the first downloaded configuration already has the selected DNS policy.
    let original = source(&client, network, token).await?;
    let mut input = NetworkConfigurationInput::from(&original.configuration);
    input.address_pools.ipv4 = Input::Value(vec!["100.64.0.0/10".into()]);
    input.address_pools.ipv6 = Input::Value(vec!["fc00::/64".into()]);
    input.dns.magic_dns = cli.dns_backend.enabled();
    input.dns.servers.clear();
    input.dns.default.clear();
    input.dns.routes.clear();
    input.dns.search_domains.clear();
    input.dns.override_local = false;
    if cli.dns_backend.enabled() {
        crate::dns::configure(&mut input, underlay.controller)?;
    }
    input.defaults.topology = TopologyDefault::Deny;
    input.defaults.dialers = match cli.transport {
        crate::CarrierMode::NativeTcp => DialerSet {
            native_udp: None,
            native_tcp: Some(NativeTcpDialer::default()),
            vless: None,
            trojan: None,
        },
        crate::CarrierMode::NativeUdp => DialerSet {
            native_udp: Some(NativeUdpDialer::default()),
            native_tcp: None,
            vless: None,
            trojan: None,
        },
    };
    apply(&client, network, token, original.revision, &input).await?;
    signals.write("enroll", &Signal::Ready).await?;
    tracing::info!(
        phase = "enrollment",
        "approving three independent daemon-generated keys"
    );
    let mut nodes = BTreeMap::new();
    let mut keys = Vec::new();
    let mut dataplane_keys = Vec::new();
    let mut bindings = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Status {
            role: claimed,
            status,
        } = signals.wait(&format!("enrollment-{}", role.name())).await?
        else {
            return Err("invalid enrollment status".into());
        };
        if claimed != role {
            return Err("enrollment role mismatch".into());
        }
        let enrollment = status
            .enrollment_request_id
            .ok_or("daemon has no pending enrollment")?;
        let actual = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                token,
            )
            .await?;
        if actual.request.hostname != role.name() || keys.contains(&actual.request.control_key) {
            return Err("enrollment name or independent key mismatch".into());
        }
        keys.push(actual.request.control_key.clone());
        client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: enrollment,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                token,
            )
            .await?;
        let Signal::Status {
            role: claimed,
            status,
        } = signals.wait(&format!("enrolled-{}", role.name())).await?
        else {
            return Err("invalid enrolled status".into());
        };
        let identity = status
            .identity
            .ok_or("daemon identity absent after approval")?;
        if claimed != role
            || identity.network_id != network
            || nodes.values().any(|node| *node == identity.node_id)
            || identity.keys.control.public_key != actual.request.control_key
            || dataplane_keys.contains(&identity.keys.dataplane.public_key)
        {
            return Err("registered identity binding mismatch".into());
        }
        dataplane_keys.push(identity.keys.dataplane.public_key.clone());
        bindings.insert(role, identity.keys);
        nodes.insert(role, identity.node_id);
    }
    let inventory = client
        .list_nodes(
            &ListNodesPath {
                network_id: network,
            },
            token,
        )
        .await?;
    if inventory.items.len() != 3
        || Role::ALL.iter().any(|role| {
            !inventory.items.iter().any(|node| {
                node.id == nodes[role]
                    && node.hostname == role.name()
                    && node.keys == bindings[role]
            })
        })
    {
        return Err("unexpected registered inventory".into());
    }
    let original = source(&client, network, token).await?;
    input = NetworkConfigurationInput::from(&original.configuration);
    for role in Role::ALL {
        let node = input
            .nodes
            .get_mut(&nodes[&role])
            .ok_or("missing enrolled node")?;
        node.listeners = BTreeMap::from([(
            "tcp".into(),
            match cli.transport {
                crate::CarrierMode::NativeTcp => Listener::NativeTcp(NativeTcpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35000.try_into()?,
                    endpoints: vec![Endpoint {
                        host: underlay.actors[&role].to_string(),
                        port: 35000.try_into()?,
                    }],
                    bind_interface: String::new(),
                    socket: Default::default(),
                    tcp: Default::default(),
                }),
                crate::CarrierMode::NativeUdp => Listener::NativeUdp(NativeUdpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35000.try_into()?,
                    endpoints: if cli.traversal == crate::TraversalMode::Nat {
                        vec![]
                    } else {
                        vec![Endpoint {
                            host: underlay.actors[&role].to_string(),
                            port: 35000.try_into()?,
                        }]
                    },
                    bind_interface: String::new(),
                    socket: Default::default(),
                }),
            },
        )]);
        if cli.traversal_fallback && role == Role::R {
            node.listeners.insert(
                "fallback-tcp".into(),
                Listener::NativeTcp(NativeTcpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35001.try_into()?,
                    endpoints: vec![Endpoint {
                        host: underlay.routers[&Role::R].wan.to_string(),
                        port: 35001.try_into()?,
                    }],
                    bind_interface: String::new(),
                    socket: Default::default(),
                    tcp: Default::default(),
                }),
            );
        }
        node.dialers = if cli.traversal_fallback && role == Role::A {
            // Only A-R owns the TCP alternative. A globally inherited TCP
            // dialer would also let B choose R's newly reachable listener after
            // withdrawal/recovery, defeating this fixture's R-B UDP constraint.
            let mut dialers = input.defaults.dialers.clone();
            dialers.native_tcp = Some(NativeTcpDialer::default());
            Input::Value(dialers)
        } else {
            Input::Omitted
        };
    }
    input.topology = [(Role::A, Role::R), (Role::R, Role::B)]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: String::new(),
            action: TopologyAction::Allow,
            a: format!("node:{}", nodes[&a]),
            b: format!("node:{}", nodes[&b]),
        })
        .collect();
    input.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", nodes[&Role::A])],
        dst: vec![format!("node:{}", nodes[&Role::B])],
        ip: vec!["udp:42424".into(), "tcp:42425".into(), "udp:42428".into()],
        bidirectional: false,
    }];
    input.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", nodes[&Role::A])],
        dst: vec![format!("node:{}", nodes[&Role::B])],
        via: vec![format!("node:{}", nodes[&Role::R])],
    }];
    let revision = apply(&client, network, token, original.revision, &input).await?;
    let committed = source(&client, network, token).await?;
    let mut addresses = BTreeMap::new();
    for role in Role::ALL {
        let node = committed
            .configuration
            .nodes
            .get(&nodes[&role])
            .ok_or("committed node missing")?;
        if cli.traversal_fallback {
            let dialers = match &node.dialers {
                Input::Value(dialers) => dialers,
                Input::Omitted => &committed.configuration.defaults.dialers,
            };
            if dialers.native_udp.is_none() || dialers.native_tcp.is_some() != (role == Role::A) {
                return Err("TCP fallback must be authorized only on the A-R edge".into());
            }
        }
        if cli.traversal == crate::TraversalMode::Nat
            && (node.listeners.len() != if cli.traversal_fallback && role == Role::R { 2 } else { 1 }
                || !node.listeners.iter().all(|(key, listener)| {
                    matches!(listener.declaration(),
                    Listener::NativeUdp(listener) if listener.enabled
                        && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                        && listener.listen_port.get() == 35000
                        && listener.endpoints.is_empty())
                    || (cli.traversal_fallback && role == Role::R && key == "fallback-tcp"
                        && matches!(listener.declaration(), Listener::NativeTcp(listener)
                            if listener.enabled && listener.listen_port.get() == 35001
                                && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                                && listener.endpoints == vec![Endpoint { host: underlay.routers[&Role::R].wan.to_string(), port: 35001.try_into().expect("fixed port") }]))
                }))
        {
            return Err(
                "committed traversal listeners contain static endpoints or another carrier".into(),
            );
        }
        addresses.insert(
            role,
            Addresses {
                ipv4: node.addresses.ipv4.ok_or("missing IPv4")?,
                ipv6: node.addresses.ipv6.ok_or("missing IPv6")?,
            },
        );
    }
    if cli.traversal == crate::TraversalMode::Nat {
        tokio::fs::write(
            cli.evidence.join("configuration-traversal.json"),
            serde_json::to_vec_pretty(&committed)?,
        )
        .await?;
    }
    signals
        .write(
            "go",
            &Signal::Configuration {
                revision,
                addresses,
                dns_domain: committed.configuration.dns.domain.clone(),
            },
        )
        .await?;
    applied(cli, &client, network, token, &nodes, revision, "initial").await?;
    if cli.traversal == crate::TraversalMode::Nat {
        signals.ready("traversal-first").await?;
        measured_links(
            cli,
            &client,
            network,
            token,
            &nodes,
            false,
            "initial-carrier",
        )
        .await?;
        audit.verify(cli, &nodes).await?;
        audit.save(cli).await?;
        signals.write("traversal-verified", &Signal::Ready).await?;
    }
    if cli.traversal_fallback {
        // Three additional demand publications and the receive-only
        // subscription warm up sequentially. Budget their aggregate rendezvous,
        // not a longer UDP exchange, signed attempt, or application lifetime.
        if !matches!(
            signals
                .wait_for("persistent-start", Duration::from_secs(90))
                .await?,
            Signal::Ready
        ) {
            return Err("invalid persistent traffic readiness".into());
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
        // Both carriers were authorized before demand creation. Only the owned
        // underlay changes here; application exchanges continue concurrently.
        signals.write("enable-fallback", &Signal::Ready).await?;
        signals.ready("fallback-router-r").await?;
        signals.ready("fallback-router-a").await?;
        measured_links(cli, &client, network, token, &nodes, true, "fallback").await?;
        signals.write("fallback-verified", &Signal::Ready).await?;
    }
    tracing::info!(
        phase = "traffic",
        "waiting for persistent UDP/TCP beyond original demand lifetime"
    );
    if !matches!(
        signals
            .wait_for("traffic", Duration::from_secs(250))
            .await?,
        Signal::Ready
    ) {
        return Err("invalid traffic completion".into());
    }
    for role in Role::ALL {
        signals
            .ready(&format!("traffic-ready-{}", role.name()))
            .await?;
    }
    measured_links(
        cli,
        &client,
        network,
        token,
        &nodes,
        cli.traversal_fallback,
        "traffic",
    )
    .await?;
    let original = source(&client, network, token).await?;
    let mut allowed = NetworkConfigurationInput::from(&original.configuration);
    // The relay no longer owns a Mesh IPv6 address, but must still forward
    // authorized IPv6 ciphertext and retain negative IPv6 capture locally.
    allowed
        .nodes
        .get_mut(&nodes[&Role::R])
        .ok_or("relay configuration absent")?
        .addresses
        .ipv6 = Input::Value(None);
    let mut denied = allowed.clone();
    denied.access.clear();
    let revision = apply(&client, network, token, original.revision, &denied).await?;
    signals
        .write("revoked", &Signal::Revision { revision })
        .await?;
    applied(cli, &client, network, token, &nodes, revision, "revoked").await?;
    signals.ready("revoked-traffic").await?;
    tracing::info!(
        phase = "access-revoked",
        "Server NodeState applied and OS echo rejection verified"
    );
    let revision = apply(&client, network, token, revision, &allowed).await?;
    signals
        .write("restored", &Signal::Revision { revision })
        .await?;
    applied(cli, &client, network, token, &nodes, revision, "restored").await?;
    for role in Role::ALL {
        signals
            .ready(&format!("lease-ready-{}", role.name()))
            .await?;
    }
    if cli.traversal_fallback {
        measured_links(
            cli,
            &client,
            network,
            token,
            &nodes,
            true,
            "restored-carrier",
        )
        .await?;
    }
    Ok(())
}

async fn measured_links(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    fallback: bool,
    stage: &str,
) -> Result<()> {
    tokio::time::timeout(STARTUP, async {
        for role in Role::ALL {
            loop {
                let view = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[&role],
                        },
                        token,
                    )
                    .await?;
                let state = view
                    .nodes
                    .get(&nodes[&role])
                    .ok_or("missing traffic NodeState")?;
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-attempt-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                let Some(VisibleReport::Full { report }) = &state.report else {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                };
                let peers: &[Role] = match role {
                    Role::A | Role::B => &[Role::R],
                    Role::R => &[Role::A, Role::B],
                };
                let complete = state.freshness == Freshness::Current
                    && peers.iter().all(|peer| {
                        report.links.get(&nodes[peer]).is_some_and(|links| {
                            links.iter().any(|link| {
                                link.phase == meshway_model::state::LinkPhase::Authenticated
                                    && link.transport
                                        == expected_carrier(cli, role, *peer, fallback).as_str()
                                    && link.business_dependencies > 0
                            }) && links.iter().all(|link| {
                                link.phase != meshway_model::state::LinkPhase::Authenticated
                                    || link.business_dependencies == 0
                                    || link.transport
                                        == expected_carrier(cli, role, *peer, fallback).as_str()
                            })
                        })
                    });
                if report.links.iter().any(|(peer, links)| {
                    !peers.iter().any(|role| nodes[role] == *peer)
                        && links.iter().any(|link| {
                            link.phase == meshway_model::state::LinkPhase::Authenticated
                        })
                }) {
                    return Err("unexpected authenticated edge bypasses mandatory relay".into());
                }
                if !complete {
                    // Installation, live adjacency and reported facts have independent
                    // lifetimes. Wait for a fresh real report after physical reattachment;
                    // rereading a report never creates or extends the observation.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                if cli.traversal == crate::TraversalMode::Nat {
                    crate::traversal::verify_state(cli, role, report).await?;
                }
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                break;
            }
        }
        Ok(())
    })
    .await?
}

fn expected_carrier(
    cli: &Cli,
    role: Role,
    peer: Role,
    fallback: bool,
) -> meshway_transport_spec::TransportProtocol {
    if cli.traversal_fallback
        && fallback
        && matches!((role, peer), (Role::A, Role::R) | (Role::R, Role::A))
    {
        meshway_transport_spec::TransportProtocol::NativeTcp
    } else {
        cli.transport.protocol()
    }
}

async fn applied(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
    revision: Revision,
    stage: &str,
) -> Result<()> {
    tokio::time::timeout(STARTUP, async {
        for role in Role::ALL {
            loop {
                let view = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[&role],
                        },
                        token,
                    )
                    .await?;
                let state = view
                    .nodes
                    .get(&nodes[&role])
                    .ok_or("missing Server NodeState")?;
                tokio::fs::write(
                    cli.evidence
                        .join(format!("state-{stage}-attempt-{}.json", role.name())),
                    serde_json::to_vec_pretty(&view)?,
                )
                .await?;
                if let Some(VisibleReport::Full { report }) = &state.report
                    && report
                        .configuration
                        .saved
                        .as_ref()
                        .is_some_and(|saved| saved.revision == revision)
                    && report
                        .configuration
                        .applied
                        .as_ref()
                        .is_some_and(|applied| applied.revision == revision)
                    && state.freshness == Freshness::Current
                {
                    tokio::fs::write(
                        cli.evidence
                            .join(format!("state-{stage}-{}.json", role.name())),
                        serde_json::to_vec_pretty(&view)?,
                    )
                    .await?;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        Ok::<(), Error>(())
    })
    .await?
}
