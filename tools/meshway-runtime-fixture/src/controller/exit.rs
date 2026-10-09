//! Exit configuration is submitted through the generated management client.
use super::*;
use crate::exit::{Role as ExitRole, Topology, read, ready, wait, write};
use meshway_configuration::{ProfileAssignment, RouteAction, RoutingProfile};

pub(super) async fn coordinate(
    cli: &Cli,
    availability: &crate::recovery::Availability,
) -> Result<()> {
    let topology: Topology = read(&cli.public.join("exit-topology.json")).await?;
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
            email: "exit-owner@example.test".into(),
            password: WireSecret::new(format!("exit-{}", request()?)),
        })
        .await?;
    let token = session.token.expose();
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: request()?,
                name: "Exit acceptance".into(),
            },
            token,
        )
        .await?
        .network
        .id;
    let original = source(&client, network, token).await?;
    let mut input = NetworkConfigurationInput::from(&original.configuration);
    input.dns.magic_dns = false;
    input.dns.override_local = false;
    input.dns.servers.clear();
    input.dns.default.clear();
    input.dns.routes.clear();
    input.dns.search_domains.clear();
    input.defaults.topology = TopologyDefault::Deny;
    input.defaults.dialers = DialerSet {
        native_udp: Some(NativeUdpDialer::default()),
        native_tcp: Some(NativeTcpDialer::default()),
        vless: None,
        trojan: None,
    };
    apply(&client, network, token, original.revision, &input).await?;
    ready(cli, "exit-enroll").await?;
    let mut nodes = BTreeMap::new();
    let mut keys = Vec::new();
    for role in ExitRole::NODES {
        let name = format!("exit-enrollment-{}.json", role.name());
        wait(cli, &name).await?;
        let status: meshway_ipc::Status = read(&cli.signals.join(name)).await?;
        let id = status
            .enrollment_request_id
            .ok_or("missing exit enrollment")?;
        let actual = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: id,
                },
                token,
            )
            .await?;
        if actual.request.hostname != role.name() || keys.contains(&actual.request.control_key) {
            return Err("exit identity mismatch".into());
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
        let name = format!("exit-enrolled-{}.json", role.name());
        wait(cli, &name).await?;
        let status: meshway_ipc::Status = read(&cli.signals.join(name)).await?;
        let identity = status.identity.ok_or("missing exit identity")?;
        if identity.network_id != network
            || identity.keys.control.public_key != actual.request.control_key
            || nodes.values().any(|n| *n == identity.node_id)
        {
            return Err("exit identity binding mismatch".into());
        }
        nodes.insert(role.name(), identity.node_id);
    }
    let original = source(&client, network, token).await?;
    input = NetworkConfigurationInput::from(&original.configuration);
    for role in ExitRole::NODES {
        let node = input
            .nodes
            .get_mut(&nodes[role.name()])
            .ok_or("exit node missing")?;
        node.exit_enabled = role == ExitRole::E;
        let links: &[&str] = match role {
            ExitRole::A => &["ar"],
            ExitRole::R => &["ar", "re"],
            _ => &["re"],
        };
        let endpoints: Vec<_> = links
            .iter()
            .map(|link| Endpoint {
                host: topology.addresses[*link][role.name()].to_string(),
                port: 35000.try_into().expect("fixed port"),
            })
            .collect();
        node.listeners = BTreeMap::from([
            (
                "udp".into(),
                Listener::NativeUdp(NativeUdpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35000.try_into()?,
                    endpoints: endpoints.clone(),
                    bind_interface: String::new(),
                    socket: Default::default(),
                }),
            ),
            (
                "tcp".into(),
                Listener::NativeTcp(NativeTcpListener {
                    enabled: true,
                    remark: String::new(),
                    listen: "0.0.0.0".parse()?,
                    listen_port: 35000.try_into()?,
                    endpoints,
                    bind_interface: String::new(),
                    socket: Default::default(),
                    tcp: Default::default(),
                }),
            ),
        ]);
    }
    input.topology = [("a", "r"), ("r", "e")]
        .into_iter()
        .map(|(a, b)| TopologyRule {
            remark: "mandatory relay".into(),
            action: TopologyAction::Allow,
            a: format!("node:{}", nodes[a]),
            b: format!("node:{}", nodes[b]),
            transports: Input::Omitted,
        })
        .collect();
    input.access = vec![AccessRule {
        remark: "exact measured Internet and exit LAN services".into(),
        src: vec![format!("node:{}", nodes["a"])],
        dst: vec!["autogroup:internet".into(), topology.lan_cidr.clone()],
        ip: vec![
            "tcp:8080".into(),
            "udp:8080".into(),
            "udp:8081".into(),
            "icmp:*".into(),
        ],
        bidirectional: false,
    }];
    input.routing.mesh.clear();
    input.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![],
            fallback: RouteAction::Exit {
                nodes: vec![format!("node:{}", nodes["e"])],
                via: vec![format!("node:{}", nodes["r"])],
            },
        },
    );
    input.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{}", nodes["a"])],
        allowed: vec!["exit".into(), "mesh".into()],
        default: "exit".into(),
        locked: false,
    }];
    let revision = apply(&client, network, token, original.revision, &input).await?;
    write(
        &cli.evidence.join("configuration.json"),
        &source(&client, network, token).await?,
    )
    .await?;
    write(&cli.signals.join("exit-configured.json"), &revision).await?;
    let mut outage = false;
    let mut withdrawn = false;
    loop {
        if tokio::fs::try_exists(cli.signals.join("exit-stop")).await? {
            break;
        }
        for role in ExitRole::NODES {
            let state = client
                .get_node_state(
                    &GetNodeStatePath {
                        network_id: network,
                        node_id: nodes[role.name()],
                    },
                    token,
                )
                .await?;
            write(
                &cli.evidence.join(format!("state-{}.json", role.name())),
                &state,
            )
            .await?;
        }
        if !outage && tokio::fs::try_exists(cli.signals.join("exit-http-close")).await? {
            write(
                &cli.signals.join("exit-http-closed.json"),
                &availability.close().await?,
            )
            .await?;
            outage = true;
        }
        if !withdrawn && tokio::fs::try_exists(cli.signals.join("exit-withdraw")).await? {
            let current = source(&client, network, token).await?;
            input
                .nodes
                .get_mut(&nodes["e"])
                .ok_or("missing E")?
                .exit_enabled = false;
            let revision = apply(&client, network, token, current.revision, &input).await?;
            write(&cli.signals.join("exit-withdrawn.json"), &revision).await?;
            withdrawn = true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}
