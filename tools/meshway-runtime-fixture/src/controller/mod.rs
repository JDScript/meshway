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
mod lifecycle;
mod links;
mod scenario;
mod server;
mod streaming;

use links::{applied, measured_links};
use server::run_server;

pub async fn run(cli: &Cli) -> Result<()> {
    let dns = crate::dns::Upstream::start(cli).await?;
    let result = run_server(cli).await;
    let dns_result = dns.shutdown(cli).await;
    result.and(dns_result)
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
    server: &server::Server<'_>,
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
    let mut input = original.configuration.clone();
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
    input = original.configuration.clone();
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
    if cli.netmap_scenario.is_some() {
        // Every Node's netmap references one inline rule-set artifact by
        // digest (a public direct rule for a documentation prefix that no
        // traffic uses); the lifecycle rows check it is fetched and cached.
        input.rule_sets.insert(
            "fixture-direct".into(),
            serde_json::from_value(serde_json::json!({
                "type": "inline", "ipCIDR": ["198.51.100.0/24"]
            }))?,
        );
        input.routing.egress.rules = vec![meshway_configuration::EgressRule {
            remark: "rule-set artifact by digest".into(),
            matcher: meshway_configuration::RouteMatch {
                rule_sets: vec!["fixture-direct".into()],
                ..Default::default()
            },
            action: meshway_configuration::RuleAction::Direct,
        }];
    }
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
            matches!(listener, Listener::NativeUdp(listener) if listener.endpoints.is_empty())) {
            return Err("reported-only fixture must not configure an endpoint".into());
        }
        let extra_tcp =
            (cli.traversal_fallback && role == Role::R) || (cli.on_demand_nat() && role == Role::A);
        if cli.traversal == crate::TraversalMode::Nat
            && (node.listeners.len() != if extra_tcp { 2 } else { 1 }
                || !node.listeners.iter().all(|(key, listener)| {
                    matches!(listener,
                    Listener::NativeUdp(listener) if listener.enabled
                        && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                        && listener.listen_port.get() == 35000
                        && listener.endpoints.is_empty())
                    || (cli.on_demand_nat() && role == Role::A && key == "reverse-tcp"
                        && matches!(listener, Listener::NativeTcp(listener)
                            if listener.enabled && listener.listen_port.get() == 35001
                                && listener.listen == std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
                                && listener.endpoints.is_empty()))
                    || (cli.traversal_fallback && role == Role::R && key == "fallback-tcp"
                        && matches!(listener, Listener::NativeTcp(listener)
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
                ipv4: match node.addresses.ipv4 {
                    Input::Value(Some(address)) => address,
                    _ => return Err("missing IPv4".into()),
                },
                ipv6: match node.addresses.ipv6 {
                    Input::Value(Some(address)) => address,
                    _ => return Err("missing IPv6".into()),
                },
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
    if cli.traversal == crate::TraversalMode::Nat {
        // Routers poll for `traffic` once a second. Their live kernel snapshot
        // belongs to the traffic phase, so the revocation below must not
        // retire its carriers first.
        signals.ready("traffic-router-a").await?;
        signals.ready("traffic-router-r").await?;
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
                server,
            },
        )
        .await;
    }
    lifecycle::run(scenario::Context {
        cli,
        signals: &signals,
        client: &client,
        network,
        token,
        nodes: &nodes,
        revision,
        availability,
        server,
    })
    .await
}
