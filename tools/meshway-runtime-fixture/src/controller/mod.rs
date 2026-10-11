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
use meshway_transport::spec::{
    DialerSet, Endpoint, Listener, NativeTcpDialer, NativeTcpListener, NativeUdpDialer,
    NativeUdpListener,
};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

mod exit;
mod interface_change;
mod scenario;
mod streaming;

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
    settings.netmap.validity = cli.netmap_validity.as_std();
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
    let availability = crate::recovery::Availability::new();
    let requests = crate::control_requests::Audit::new();
    let mut router = meshway_server::http::router(&app);
    if cli.traversal == crate::TraversalMode::Nat {
        router = router.layer(axum::middleware::from_fn_with_state(
            audit.clone(),
            crate::traversal::observe,
        ));
    }
    router = router.layer(axum::middleware::from_fn_with_state(
        availability.clone(),
        crate::recovery::gate,
    ));
    router = router.layer(axum::middleware::from_fn_with_state(
        requests.clone(),
        crate::control_requests::observe,
    ));
    if cli.traversal == crate::TraversalMode::Exit {
        // The hold precedes admission so closing the gate can drain already
        // admitted database work independently of deliberately held requests.
        router = router.layer(axum::middleware::from_fn_with_state(
            (cli.evidence.clone(), cli.signals.clone()),
            exit::observe_acquisition,
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
    let result = tokio::time::timeout(DEADLINE, async {
        if cli.traversal == crate::TraversalMode::Exit {
            exit::coordinate(cli, &availability).await
        } else {
            coordinate(cli, &audit, &availability).await
        }
    })
    .await;
    let audit_result = audit.save(cli).await;
    let request_result = requests.save(cli).await;
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
    request_result?;
    result??;
    if cli.traversal == crate::TraversalMode::Exit {
        return Ok(());
    }
    let signals = Signals::new(&cli.signals);
    signals.write("offline", &Signal::Ready).await?;
    tracing::info!(
        phase = "controller-offline",
        "actual listener and Server owner stopped"
    );
    if let Some(netmap_scenario) = cli.netmap_scenario {
        for role in Role::ALL {
            let Signal::Observed {
                role: claimed,
                at_ms,
            } = signals
                .wait_for(
                    &format!("scenario-down-{}", role.name()),
                    Duration::from_secs(90),
                )
                .await?
            else {
                return Err("invalid scenario down evidence".into());
            };
            if claimed != role {
                return Err("scenario down role mismatch".into());
            }
            #[derive(serde::Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Report {
                scenario: crate::NetmapScenario,
                role: Role,
                down_at_ms: u64,
            }
            tokio::fs::write(
                cli.evidence.join(format!("report-{}.json", role.name())),
                serde_json::to_vec_pretty(&Report {
                    scenario: netmap_scenario,
                    role,
                    down_at_ms: at_ms,
                })?,
            )
            .await?;
        }
        return Ok(());
    }
    for role in Role::ALL {
        let report = signals
            .wait_for(&format!("report-{}", role.name()), Duration::from_secs(90))
            .await?;
        validate_report(&report, role, cli.dns_backend, cli.traversal_fallback)?;
        tokio::fs::write(
            cli.evidence.join(format!("report-{}.json", role.name())),
            serde_json::to_vec_pretty(&report)?,
        )
        .await?;
    }
    Ok(())
}

/// Final report assertions shared with retained-artifact inspection. Decoding
/// an old report never changes its run verdict or substitutes for a fresh run.
pub(super) fn validate_report(
    report: &Signal,
    role: Role,
    dns_backend: crate::DnsBackend,
    traversal_fallback: bool,
) -> Result<()> {
    match report {
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
                    traversal_fallback
                        && role == Role::A
                        && proof.passed
                        && proof.circuits.len() == 1
                }
                None => !(traversal_fallback && role == Role::A),
            }
            && (dns.as_ref().is_some_and(|evidence| {
                evidence.backend == dns_backend
                    && evidence.original_resolv_conf_restored == Some(true)
                    && evidence.original_links_restored == Some(true)
            }) == dns_backend.enabled())
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
    if let Signal::Report {
        node,
        recovery,
        ping,
        ..
    } = report
    {
        match (role, ping) {
            (Role::A, Some(ping)) => ping.validate(*node, recovery)?,
            (Role::R | Role::B, None) => {}
            _ => return Err("ping evidence must be present only for source actor A".into()),
        }
        recovery.validate(role)?;
        if recovery
            .recovered
            .identity
            .as_ref()
            .is_none_or(|identity| identity.node_id != *node)
        {
            return Err("final recovery report is bound to a different Node".into());
        }
    }
    Ok(())
}

/// Every actor recorded its own daemon at this stage's revision. The next
/// revision is published only afterwards: an actor polling its status must
/// never be asked for a revision its daemon has already replaced.
async fn staged(signals: &Signals, stage: &str) -> Result<()> {
    for role in Role::ALL {
        match signals
            .wait_for(&format!("{stage}-{}", role.name()), STARTUP + IO)
            .await?
        {
            Signal::Status { role: claimed, .. } if claimed == role => {}
            _ => return Err(format!("invalid {stage} status rendezvous").into()),
        }
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

async fn coordinate(
    cli: &Cli,
    audit: &crate::traversal::Audit,
    availability: &crate::recovery::Availability,
) -> Result<()> {
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
        crate::CarrierMode::Vless => DialerSet {
            native_udp: None,
            native_tcp: None,
            vless: Some(Default::default()),
            trojan: None,
        },
        crate::CarrierMode::Trojan => DialerSet {
            native_udp: None,
            native_tcp: None,
            vless: None,
            trojan: Some(Default::default()),
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
        node.listeners = if cli.transport.streaming() {
            if role == Role::R {
                BTreeMap::from([(
                    "stream".into(),
                    streaming::listener(cli, underlay.actors[&role].to_string()).await?,
                )])
            } else {
                BTreeMap::new()
            }
        } else {
            BTreeMap::from([(
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
                        endpoints: if cli.traversal == crate::TraversalMode::Nat
                            || cli.reported_endpoints
                        {
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
                    crate::CarrierMode::Vless | crate::CarrierMode::Trojan => unreachable!(),
                },
            )])
        };
        if cli.on_demand_nat() && role == Role::A {
            node.listeners.insert(
                "reverse-tcp".into(),
                Listener::NativeTcp(NativeTcpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35001.try_into()?,
                    endpoints: vec![],
                    bind_interface: String::new(),
                    socket: Default::default(),
                    tcp: Default::default(),
                }),
            );
        }
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
        node.dialers = if cli.transport.streaming() {
            let mut dialers = input.defaults.dialers.clone();
            if role == Role::R {
                dialers.vless = None;
                dialers.trojan = None;
            }
            Input::Value(dialers)
        } else if (cli.traversal == crate::TraversalMode::Nat || cli.reported_endpoints)
            && role == Role::R
        {
            // Both tested carriers must enter R's private NAT. Otherwise R
            // could now dial B's reported public-side interface directly and
            // correctly avoid the bilateral STUN path this fixture proves.
            // Business still follows A-R-B over the bidirectional channels.
            // Local-endpoint conformance likewise requires A/B to redial R's
            // reported address, rather than passing through R's reverse dial.
            // The on-demand NAT case gives R only a TCP dialer: B has no TCP
            // listener, so R–B stays B→R, while A–R becomes dialable both ways.
            Input::Value(meshway_transport::spec::DialerSet {
                native_udp: None,
                native_tcp: cli.on_demand_nat().then(NativeTcpDialer::default),
                vless: None,
                trojan: None,
            })
        } else if cli.traversal_fallback && role == Role::A {
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
            transports: meshway_configuration::Input::Omitted,
            // Incoming-only R has no dialer in the reported-endpoint/NAT rows;
            // B maintains its declared B→R carrier while A→R remains lazy.
            keepalive: a == Role::R
                && (cli.reported_endpoints || cli.traversal == crate::TraversalMode::Nat),
        })
        .collect();
    input.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", nodes[&Role::A])],
        dst: vec![format!("node:{}", nodes[&Role::B])],
        ip: vec!["udp:42424".into(), "tcp:42425".into(), "udp:42428".into()],
        bidirectional: false,
    }];
    // Exact diagnostic grants are separate from the unchanged business ports.
    for target in [Role::B, Role::R] {
        input.access.push(AccessRule {
            remark: "Real CLI ICMP diagnostic".into(),
            src: vec![format!("node:{}", nodes[&Role::A])],
            dst: vec![format!("node:{}", nodes[&target])],
            ip: vec!["icmp:*".into()],
            bidirectional: false,
        });
    }
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
        if cli.transport.streaming() {
            let Input::Value(dialers) = &node.dialers else {
                return Err("streaming dialers must be explicit".into());
            };
            if dialers.native_udp.is_some()
                || dialers.native_tcp.is_some()
                || dialers.vless.is_some()
                    != (role != Role::R && matches!(cli.transport, crate::CarrierMode::Vless))
                || dialers.trojan.is_some()
                    != (role != Role::R && matches!(cli.transport, crate::CarrierMode::Trojan))
                || node.listeners.len() != usize::from(role == Role::R)
            {
                return Err(
                    "streaming fixture must use only A/B dialing R on the chosen carrier".into(),
                );
            }
        }
        if cli.traversal_fallback {
            let dialers = match &node.dialers {
                Input::Value(dialers) => dialers,
                Input::Omitted => &committed.configuration.defaults.dialers,
            };
            if dialers.native_udp.is_some() != (role != Role::R)
                || dialers.native_tcp.is_some() != (role == Role::A)
            {
                return Err("TCP fallback must be authorized only on the A-R edge".into());
            }
        }
        if cli.on_demand_nat() {
            let dialers = match &node.dialers {
                Input::Value(dialers) => dialers,
                Input::Omitted => &committed.configuration.defaults.dialers,
            };
            if dialers.native_udp.is_some() != (role != Role::R)
                || dialers.native_tcp.is_some() != (role == Role::R)
                || dialers.vless.is_some()
                || dialers.trojan.is_some()
            {
                return Err("on-demand NAT edge must let R dial A only over Native TCP".into());
            }
        }
        if cli.reported_endpoints && !node.listeners.values().all(|listener|
            matches!(listener.declaration(), Listener::NativeUdp(listener) if listener.endpoints.is_empty())) {
            return Err("reported-only fixture must not configure an endpoint".into());
        }
        let extra_tcp =
            (cli.traversal_fallback && role == Role::R) || (cli.on_demand_nat() && role == Role::A);
        if cli.traversal == crate::TraversalMode::Nat
            && (node.listeners.len() != if extra_tcp { 2 } else { 1 }
                || !node.listeners.iter().all(|(key, listener)| {
                    matches!(listener.declaration(),
                    Listener::NativeUdp(listener) if listener.enabled
                        && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                        && listener.listen_port.get() == 35000
                        && listener.endpoints.is_empty())
                    || (cli.on_demand_nat() && role == Role::A && key == "reverse-tcp"
                        && matches!(listener.declaration(), Listener::NativeTcp(listener)
                            if listener.enabled && listener.listen_port.get() == 35001
                                && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                                && listener.endpoints.is_empty()))
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
    if cli.transport.streaming() {
        tokio::fs::write(
            cli.evidence.join("configuration-streaming.json"),
            serde_json::to_vec_pretty(&committed)?,
        )
        .await?;
    }
    if cli.reported_endpoints {
        tokio::fs::write(
            cli.evidence.join("configuration-reported-endpoints.json"),
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
    applied(
        cli,
        &client,
        network,
        token,
        &nodes,
        revision,
        ("initial", None),
    )
    .await?;
    crate::ping_management::initial(cli, &signals, &client, network, token, &nodes, audit).await?;
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
        // Additional business circuits and the receive-only application
        // subscription warm up sequentially. Budget their aggregate rendezvous.
        if !matches!(
            signals
                .wait_for("persistent-start", Duration::from_secs(90))
                .await?,
            Signal::Ready
        ) {
            return Err("invalid persistent traffic readiness".into());
        }
        tokio::time::sleep(cli.fallback_delay()).await;
        // Both carriers were authorized by the initial netmap. Only the owned
        // underlay changes here; application exchanges continue concurrently.
        signals.write("enable-fallback", &Signal::Ready).await?;
        signals.ready("fallback-router-r").await?;
        signals.ready("fallback-router-a").await?;
        measured_links(cli, &client, network, token, &nodes, true, "fallback").await?;
        signals.write("fallback-verified", &Signal::Ready).await?;
    }
    tracing::info!(
        phase = "traffic",
        "waiting for persistent UDP/TCP on the same OS sockets"
    );
    if !matches!(
        signals.wait_for("traffic", cli.traffic_wait()).await?,
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
    if let Some(netmap_scenario) = cli.netmap_scenario {
        return scenario::run(
            netmap_scenario,
            scenario::Context {
                cli,
                signals: &signals,
                client: &client,
                network,
                token,
                nodes: &nodes,
                revision,
                availability,
            },
        )
        .await;
    }
    let original = source(&client, network, token).await?;
    let mut allowed = NetworkConfigurationInput::from(&original.configuration);
    // Isolate ICMP revocation from the later relay address change: changing a
    // Node's addresses intentionally invalidates its execution continuity.
    let mut business_only = allowed.clone();
    // The relay no longer owns a Mesh IPv6 address, but must still forward
    // authorized IPv6 ciphertext and retain negative IPv6 capture locally.
    allowed
        .nodes
        .get_mut(&nodes[&Role::R])
        .ok_or("relay configuration absent")?
        .addresses
        .ipv6 = Input::Value(None);
    business_only
        .access
        .retain(|rule| !rule.ip.iter().any(|ip| ip == "icmp:*"));
    if business_only.access.len() != 1
        || business_only.access[0].ip != ["tcp:42425", "udp:42424", "udp:42428"]
    {
        return Err("ICMP-only revocation widened or changed business permissions".into());
    }
    let revision = apply(&client, network, token, original.revision, &business_only).await?;
    signals
        .write("ping-icmp-revoked", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        &client,
        network,
        token,
        &nodes,
        revision,
        ("icmp-revoked", None),
    )
    .await?;
    staged(&signals, "icmp-revoked").await?;
    let Signal::PingRun { run } = signals
        .wait_for("ping-udp-tcp-only", crate::ping::STAGE_LIMIT + IO)
        .await?
    else {
        return Err("missing UDP/TCP-only diagnostic denial".into());
    };
    run.validate(&[nodes[&Role::A], nodes[&Role::R], nodes[&Role::B]])?;
    crate::ping_management::denied(cli, &client, network, token, &nodes, &run).await?;
    let mut denied = allowed.clone();
    denied.access.clear();
    let revision = apply(&client, network, token, revision, &denied).await?;
    signals
        .write("revoked", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        &client,
        network,
        token,
        &nodes,
        revision,
        ("revoked", None),
    )
    .await?;
    staged(&signals, "revoked").await?;
    if !matches!(
        signals
            .wait_for("revoked-traffic", STARTUP + crate::ping::STAGE_LIMIT)
            .await?,
        Signal::Ready
    ) {
        return Err("invalid revoked traffic/diagnostic completion".into());
    }
    let Signal::PingRun { run } = signals.wait("ping-revoked").await? else {
        return Err("missing permission-denied CLI evidence".into());
    };
    run.validate(&[nodes[&Role::A], nodes[&Role::R], nodes[&Role::B]])?;
    tracing::info!(
        phase = "access-revoked",
        "Server NodeState applied and OS echo rejection verified"
    );
    let revision = apply(&client, network, token, revision, &allowed).await?;
    signals
        .write("restored", &Signal::Revision { revision })
        .await?;
    applied(
        cli,
        &client,
        network,
        token,
        &nodes,
        revision,
        ("restored", None),
    )
    .await?;
    interface_change::run(cli, &signals, &client, network, token, &nodes).await?;
    for role in Role::ALL {
        if !matches!(
            signals
                .wait_for(
                    &format!("lease-ready-{}", role.name()),
                    STARTUP + crate::ping::STAGE_LIMIT * 2
                )
                .await?,
            Signal::Ready
        ) {
            return Err("invalid restored traffic/CLI lifecycle completion".into());
        }
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
    signals.ready("ping-outage-ready").await?;
    let closed_at_ms = availability.close().await?;
    signals
        .write(
            "http-outage",
            &Signal::HttpAvailability {
                available: false,
                observed_at_ms: closed_at_ms,
            },
        )
        .await?;
    tracing::info!(
        phase = "http-outage",
        closed_at_ms,
        "Node HTTP admission closed; real Server owner remains running"
    );
    let mut expired = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Expired {
            role: claimed,
            evidence,
        } = signals
            .wait_for(
                &format!("instance-expired-{}", role.name()),
                cli.netmap_validity.as_std() + Duration::from_secs(75),
            )
            .await?
        else {
            return Err("missing instance-expiry evidence".into());
        };
        evidence.validate()?;
        if claimed != role
            || evidence.closed_at_ms != closed_at_ms
            || evidence.before.identity.as_ref().is_none_or(|identity| {
                identity.node_id != nodes[&role] || identity.network_id != network
            })
        {
            return Err("instance-expiry evidence identity mismatch".into());
        }
        let view = client
            .get_node_state(
                &GetNodeStatePath {
                    network_id: network,
                    node_id: nodes[&role],
                },
                token,
            )
            .await?;
        if view
            .nodes
            .get(&nodes[&role])
            .is_none_or(|state| state.freshness == Freshness::Current)
        {
            return Err(
                "Server still reports current Node State after gated instance expiry".into(),
            );
        }
        tokio::fs::write(
            cli.evidence
                .join(format!("state-expired-{}.json", role.name())),
            serde_json::to_vec_pretty(&view)?,
        )
        .await?;
        expired.insert(role, evidence);
    }
    let (reopened_at_ms, rejected_requests) = availability.open()?;
    tokio::fs::write(
        cli.evidence.join("http-outage.json"),
        serde_json::to_vec_pretty(&crate::recovery::Window {
            closed_at_ms,
            reopened_at_ms,
            rejected_node_requests: rejected_requests,
            response_status: 503,
        })?,
    )
    .await?;
    signals
        .write(
            "recovery-open",
            &Signal::HttpAvailability {
                available: true,
                observed_at_ms: reopened_at_ms,
            },
        )
        .await?;
    let mut recovered = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Status {
            role: claimed,
            status,
        } = signals
            .wait_for(
                &format!("recovery-applied-{}", role.name()),
                Duration::from_secs(75),
            )
            .await?
        else {
            return Err("missing automatically recovered status".into());
        };
        if claimed != role {
            return Err("recovered role mismatch".into());
        }
        expired[&role].validate_recovered(&status, reopened_at_ms)?;
        recovered.insert(role, *status);
    }
    applied(
        cli,
        &client,
        network,
        token,
        &nodes,
        revision,
        ("recovered", Some(&recovered)),
    )
    .await?;
    signals
        .write("recovery-state-verified", &Signal::Ready)
        .await?;
    for role in Role::ALL {
        if !matches!(
            signals
                .wait_for(
                    &format!("recovery-done-{}", role.name()),
                    Duration::from_secs(150) + crate::ping::STAGE_LIMIT
                )
                .await?,
            Signal::Ready
        ) {
            return Err("invalid recovery traffic completion".into());
        }
    }
    measured_links(
        cli,
        &client,
        network,
        token,
        &nodes,
        cli.traversal_fallback,
        "recovered",
    )
    .await?;
    tracing::info!(
        phase = "automatic-recovery-verified",
        "New instances, real applied State and fresh OS flows verified; stopping actual Server next"
    );
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
                    && report.configuration.applied.is_some()
                    && report.configuration.applied == report.configuration.saved
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
) -> meshway_model::TransportProtocol {
    if cli.traversal_fallback
        && fallback
        && matches!((role, peer), (Role::A, Role::R) | (Role::R, Role::A))
    {
        meshway_model::TransportProtocol::NativeTcp
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
    observation: (&str, Option<&BTreeMap<Role, meshway_protocol::ipc::Status>>),
) -> Result<()> {
    let (stage, expected) = observation;
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
                    && expected.is_none_or(|expected| {
                        expected.get(&role).is_some_and(|status| {
                            state.instance_id
                                == status
                                    .instance
                                    .as_ref()
                                    .map(|instance| instance.instance_id)
                                && report.configuration.applied == report.configuration.saved
                        })
                    })
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
