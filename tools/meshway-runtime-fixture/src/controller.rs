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
use meshway_transport_spec::{DialerSet, Endpoint, Listener, NativeTcpDialer, NativeTcpListener};
use std::{collections::BTreeMap, net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

pub async fn run(cli: &Cli) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8443").await?;
    let tls = meshway_packet_fixture::tls_server(&cli.public, &cli.private).await?;
    let app = App::open(Settings::new(cli.state.join("server"))).await?;
    let router = meshway_server::http::router(&app);
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
                            let stream = tls.accept(stream).await.map_err(|_| "controller TLS handshake")?;
                            let service = hyper_util::service::TowerToHyperService::new(router);
                            hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(stream), service).await
                                .map_err(|_| "controller HTTP connection")?;
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
    let result = tokio::time::timeout(DEADLINE, coordinate(cli)).await;
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
                ..
            } if *claimed == role
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

async fn coordinate(cli: &Cli) -> Result<()> {
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
    // particular the first downloaded configuration already has DNS disabled.
    let original = source(&client, network, token).await?;
    let mut input = NetworkConfigurationInput::from(&original.configuration);
    input.address_pools.ipv4 = Input::Value(vec!["100.64.0.0/10".into()]);
    input.address_pools.ipv6 = Input::Value(vec!["fc00::/64".into()]);
    input.dns.magic_dns = false;
    input.dns.servers.clear();
    input.dns.default.clear();
    input.dns.routes.clear();
    input.dns.search_domains.clear();
    input.dns.override_local = false;
    input.defaults.topology = TopologyDefault::Deny;
    input.defaults.dialers = DialerSet {
        native_tcp: Some(NativeTcpDialer::default()),
        ..DialerSet::default()
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
            Listener::NativeTcp(NativeTcpListener {
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
        )]);
        node.dialers = Input::Omitted;
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
        addresses.insert(
            role,
            Addresses {
                ipv4: node.addresses.ipv4.ok_or("missing IPv4")?,
                ipv6: node.addresses.ipv6.ok_or("missing IPv6")?,
            },
        );
    }
    signals
        .write(
            "go",
            &Signal::Configuration {
                revision,
                addresses,
            },
        )
        .await?;
    applied(cli, &client, network, token, &nodes, revision, "initial").await?;
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
    measured_links(cli, &client, network, token, &nodes).await?;
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
    Ok(())
}

async fn measured_links(
    cli: &Cli,
    client: &Client,
    network: NetworkId,
    token: &str,
    nodes: &BTreeMap<Role, NodeId>,
) -> Result<()> {
    for role in Role::ALL {
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
        let Some(VisibleReport::Full { report }) = &state.report else {
            return Err("missing measured full NodeState".into());
        };
        if state.freshness != Freshness::Current {
            return Err("traffic NodeState is stale".into());
        }
        let peers: &[Role] = match role {
            Role::A | Role::B => &[Role::R],
            Role::R => &[Role::A, Role::B],
        };
        for peer in peers {
            if !report.links.get(&nodes[peer]).is_some_and(|links| {
                links.iter().any(|link| {
                    link.phase == meshway_model::state::LinkPhase::Authenticated
                        && link.transport == "native_tcp"
                        && link.business_dependencies > 0
                })
            }) {
                return Err(
                    "mandatory relay edge lacks measured authenticated business dependency".into(),
                );
            }
        }
        if report.links.iter().any(|(peer, links)| {
            !peers.iter().any(|role| nodes[role] == *peer)
                && links
                    .iter()
                    .any(|link| link.phase == meshway_model::state::LinkPhase::Authenticated)
        }) {
            return Err("unexpected authenticated edge bypasses mandatory relay".into());
        }
        tokio::fs::write(
            cli.evidence
                .join(format!("state-traffic-{}.json", role.name())),
            serde_json::to_vec_pretty(&view)?,
        )
        .await?;
    }
    Ok(())
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
