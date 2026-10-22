//! `fault/throttle` configuration through the generated management client:
//! A→B through the mandatory relay R, B's egress capture through its exit
//! E, MagicDNS on every Node. Static Native UDP endpoints, resident edges.
use super::*;
use crate::fault::{Configured, ROW, Role as FaultRole, Topology};
use crate::supervised::{read, ready, stopped, wait, write};
use meshway_configuration::{ProfileAssignment, RouteAction, RoutingProfile};

pub(super) async fn coordinate(cli: &Cli) -> Result<()> {
    let topology: Topology = read(&cli.public.join("fault-topology.json")).await?;
    let client = Client::new(
        meshway_packet_fixture::CONTROLLER_ORIGIN,
        meshway_packet_fixture::tls_client(&cli.public, IO)
            .await?
            .resolve(
                meshway_packet_fixture::CONTROLLER_NAME,
                "127.0.0.1:8443".parse()?,
            ),
        16 * 1024 * 1024,
    )?;
    let session = client
        .register(&RegisterRequest {
            email: "fault-owner@example.test".into(),
            password: WireSecret::new(format!("fault-{}", request()?)),
        })
        .await?;
    let token = session.token.expose();
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: request()?,
                name: "Fault injection acceptance".into(),
            },
            token,
        )
        .await?
        .network
        .id;
    let original = source(&client, network, token).await?;
    let mut input = original.configuration.clone();
    input.address_pools.ipv4 = Input::Value(vec!["100.64.0.0/10".into()]);
    input.address_pools.ipv6 = Input::Value(vec!["fc00::/64".into()]);
    // The DNS owner of every Node serves Mesh names through resolved.
    input.dns.magic_dns = true;
    input.dns.override_local = false;
    input.dns.servers.clear();
    input.dns.default.clear();
    input.dns.routes.clear();
    input.dns.search_domains.clear();
    input.defaults.topology = TopologyDefault::Deny;
    input.defaults.dialers = DialerSet {
        native_udp: Some(NativeUdpDialer::default()),
        native_tcp: None,
        vless: None,
        trojan: None,
    };
    apply(&client, network, token, original.revision, &input).await?;
    ready(cli, "fault-enroll").await?;
    let mut nodes = BTreeMap::new();
    let mut keys = Vec::new();
    for role in FaultRole::NODES {
        let name = format!("{ROW}-enrollment-{}.json", role.name());
        wait(cli, ROW, &name).await?;
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
        let id = status
            .enrollment_request_id
            .ok_or("missing fault enrollment")?;
        let actual = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: id,
                },
                token,
            )
            .await?;
        if actual.request.hostname != role.name() || keys.contains(&actual.request.control_key) {
            return Err("fault identity mismatch".into());
        }
        keys.push(actual.request.control_key.clone());
        client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: id,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                token,
            )
            .await?;
        let name = format!("{ROW}-enrolled-{}.json", role.name());
        wait(cli, ROW, &name).await?;
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
        let identity = status.identity.ok_or("missing fault identity")?;
        if identity.network_id != network
            || identity.keys.control.public_key != actual.request.control_key
            || nodes.values().any(|n| *n == identity.node_id)
        {
            return Err("fault identity binding mismatch".into());
        }
        nodes.insert(role.name().to_owned(), identity.node_id);
    }
    let node = |role: FaultRole| format!("node:{}", nodes[role.name()]);
    let original = source(&client, network, token).await?;
    input = original.configuration.clone();
    for role in FaultRole::NODES {
        let config = input
            .nodes
            .get_mut(&nodes[role.name()])
            .ok_or("fault node missing")?;
        let address = topology
            .nodes
            .get(role.name())
            .ok_or("fault node address missing")?;
        config.listeners = BTreeMap::from([(
            "udp".into(),
            Listener::NativeUdp(NativeUdpListener {
                enabled: true,
                remark: String::new(),
                listen: "0.0.0.0".parse()?,
                listen_port: 35000.try_into()?,
                endpoints: vec![Endpoint {
                    host: address.to_string(),
                    port: 35000.try_into()?,
                }],
                bind_interface: String::new(),
                socket: Default::default(),
            }),
        )]);
        config.dialers = Input::Omitted;
        config.exit_enabled = role == FaultRole::E;
    }
    // Resident edges: no idle close inside a measured window.
    input.topology = [
        (FaultRole::A, FaultRole::R),
        (FaultRole::R, FaultRole::B),
        (FaultRole::B, FaultRole::E),
    ]
    .into_iter()
    .map(|(a, b)| TopologyRule {
        remark: String::new(),
        action: TopologyAction::Allow,
        a: node(a),
        b: node(b),
        transports: Input::Omitted,
        keepalive: true,
    })
    .collect();
    let target = format!("{}/32", topology.target);
    input.access = vec![
        AccessRule {
            remark: "business flow through R".into(),
            src: vec![node(FaultRole::A)],
            dst: vec![node(FaultRole::B)],
            ip: vec!["udp:42424".into(), "icmp:*".into()],
            // Both ways: B resolves A's Mesh name after each resolved restart.
            bidirectional: true,
        },
        AccessRule {
            remark: "B's exit target".into(),
            src: vec![node(FaultRole::B)],
            dst: vec![target.clone()],
            ip: vec!["udp:42424".into()],
            bidirectional: false,
        },
    ];
    input.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![node(FaultRole::A)],
        dst: vec![node(FaultRole::B)],
        via: vec![node(FaultRole::R)],
    }];
    // B's egress capture: one IP rule through E, no DNS plan.
    input.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![meshway_configuration::EgressRule {
                remark: "target through E".into(),
                matcher: meshway_configuration::RouteMatch {
                    ip_cidr: vec![target],
                    ..Default::default()
                },
                action: meshway_configuration::RuleAction::Exit {
                    nodes: vec![node(FaultRole::E)],
                    via: Vec::new(),
                },
            }],
            fallback: RouteAction::Direct,
        },
    );
    input.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![node(FaultRole::B)],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    }];
    let revision = apply(&client, network, token, original.revision, &input).await?;
    let committed = source(&client, network, token).await?;
    let mut mesh = BTreeMap::new();
    for role in FaultRole::NODES {
        let config = committed
            .configuration
            .nodes
            .get(&nodes[role.name()])
            .ok_or("committed fault node missing")?;
        let Input::Value(Some(address)) = config.addresses.ipv4 else {
            return Err("missing fault Mesh IPv4".into());
        };
        mesh.insert(role.name().to_owned(), address);
    }
    write(
        &cli.signals.join("fault-configured.json"),
        &Configured {
            revision,
            nodes: nodes.clone(),
            mesh,
            dns_domain: committed.configuration.dns.domain.clone(),
        },
    )
    .await?;
    // The Server answers until the supervisor stops the run: the nodes go
    // down first (`fault-down`), while it still answers.
    while !stopped(cli, ROW).await? {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}
