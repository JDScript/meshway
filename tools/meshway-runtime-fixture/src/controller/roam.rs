//! Roaming row configuration through the generated management client, and
//! the Server State the supervisor reads (circuit and channel identities).
use super::*;
use crate::roam::{
    Configured, Phase, Role as RoamRole, Topology, read, ready, stopped, wait, write,
};

/// A Native UDP listener on 35000; without a published endpoint the peer
/// reaches it only through coordinated traversal (its STUN mapping).
fn udp_listener(endpoint: Option<std::net::Ipv4Addr>) -> Result<Listener> {
    Ok(Listener::NativeUdp(NativeUdpListener {
        enabled: true,
        remark: String::new(),
        listen: "0.0.0.0".parse()?,
        listen_port: 35000.try_into()?,
        endpoints: endpoint
            .map(|endpoint| {
                Ok::<_, crate::Error>(Endpoint {
                    host: endpoint.to_string(),
                    port: 35000.try_into()?,
                })
            })
            .transpose()?
            .into_iter()
            .collect(),
        bind_interface: String::new(),
        socket: Default::default(),
    }))
}

fn dialers(udp: bool) -> Input<DialerSet> {
    Input::Value(DialerSet {
        native_udp: udp.then(NativeUdpDialer::default),
        native_tcp: None,
        vless: None,
        trojan: None,
    })
}

pub(super) async fn coordinate(cli: &Cli) -> Result<()> {
    let topology: Topology = read(&cli.public.join("roam-topology.json")).await?;
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
            email: "roam-owner@example.test".into(),
            password: WireSecret::new(format!("roam-{}", request()?)),
        })
        .await?;
    let token = session.token.expose();
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: request()?,
                name: "Roaming acceptance".into(),
            },
            token,
        )
        .await?
        .network
        .id;
    let original = source(&client, network, token).await?;
    let mut input = original.configuration.clone();
    input.address_pools.ipv4 = Input::Value(vec!["100.64.0.0/10".into()]);
    input.dns.magic_dns = false;
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
    ready(cli, "roam-enroll").await?;
    let mut nodes = BTreeMap::new();
    let mut keys = Vec::new();
    let roles = topology.scenario.nodes();
    for &role in roles {
        let name = format!("roam-enrollment-{}.json", role.name());
        wait(cli, &name).await?;
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
        let id = status
            .enrollment_request_id
            .ok_or("missing roam enrollment")?;
        let actual = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: id,
                },
                token,
            )
            .await?;
        if actual.request.hostname != role.name() || keys.contains(&actual.request.control_key) {
            return Err("roam identity mismatch".into());
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
        let name = format!("roam-enrolled-{}.json", role.name());
        wait(cli, &name).await?;
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
        let identity = status.identity.ok_or("missing roam identity")?;
        if identity.network_id != network
            || identity.keys.control.public_key != actual.request.control_key
            || nodes.values().any(|n| *n == identity.node_id)
        {
            return Err("roam identity binding mismatch".into());
        }
        nodes.insert(role.name().to_owned(), identity.node_id);
    }
    let node = |role: RoamRole| format!("node:{}", nodes[role.name()]);
    for &phase in topology.scenario.phases() {
        let original = source(&client, network, token).await?;
        input = original.configuration.clone();
        for &role in roles {
            let config = input
                .nodes
                .get_mut(&nodes[role.name()])
                .ok_or("roam node missing")?;
            // Exactly one side of each edge can dial: the edge is resident
            // and its direction is the phase's.
            // Traversal: A and B both listen without a published endpoint
            // and both dial, so only coordinated traversal joins them; R is
            // a public listener both of them dial.
            let listen = |endpoint| -> Result<_> {
                Ok(BTreeMap::from([("udp".into(), udp_listener(endpoint)?)]))
            };
            let (listeners, udp) = match (phase, role) {
                (Phase::Dialer, RoamRole::A) => (BTreeMap::new(), true),
                (Phase::Listener, RoamRole::A) => (listen(topology.a_published)?, true),
                (Phase::Dialer, RoamRole::B) => (listen(Some(topology.b))?, false),
                (Phase::Listener, RoamRole::B) => (BTreeMap::new(), true),
                (Phase::Traversal, RoamRole::A | RoamRole::B) => (listen(None)?, true),
                (_, RoamRole::R) => (listen(topology.r)?, false),
                (_, _) => (listen(Some(topology.c_lan))?, false),
            };
            config.listeners = listeners;
            config.dialers = dialers(udp);
        }
        let mut edges = vec![(RoamRole::A, RoamRole::B), (RoamRole::A, RoamRole::C)];
        if roles.contains(&RoamRole::R) {
            edges.extend([(RoamRole::A, RoamRole::R), (RoamRole::R, RoamRole::B)]);
        }
        input.topology = edges
            .into_iter()
            .map(|(a, b)| TopologyRule {
                remark: String::new(),
                action: TopologyAction::Allow,
                a: node(a),
                b: node(b),
                transports: Input::Omitted,
                // An edge only one side can dial is resident; the traversal
                // edge, dialable both ways, is made resident explicitly.
                keepalive: phase == Phase::Traversal && b == RoamRole::B && a == RoamRole::A,
            })
            .collect();
        input.access = vec![
            AccessRule {
                remark: "roaming business traffic".into(),
                src: vec![node(RoamRole::A)],
                dst: vec![node(RoamRole::B)],
                ip: vec!["udp:42424".into(), "tcp:42425".into(), "icmp:*".into()],
                bidirectional: true,
            },
            AccessRule {
                remark: "unrelated LAN channel".into(),
                src: vec![node(RoamRole::A)],
                dst: vec![node(RoamRole::C)],
                ip: vec!["udp:42424".into(), "tcp:42425".into(), "icmp:*".into()],
                bidirectional: true,
            },
        ];
        input.routing.mesh.clear();
        let revision = apply(&client, network, token, original.revision, &input).await?;
        let committed = source(&client, network, token).await?;
        let mut mesh = BTreeMap::new();
        for &role in roles {
            let config = committed
                .configuration
                .nodes
                .get(&nodes[role.name()])
                .ok_or("committed roam node missing")?;
            let Input::Value(Some(address)) = config.addresses.ipv4 else {
                return Err("missing roam Mesh IPv4".into());
            };
            mesh.insert(role.name().to_owned(), address);
        }
        write(
            &cli.signals
                .join(format!("roam-configured-{}.json", phase.name())),
            &Configured {
                revision,
                nodes: nodes.clone(),
                mesh,
            },
        )
        .await?;
        // The supervisor reads circuit and channel identities from the real
        // Server State until it ends this phase.
        let done = format!("roam-phase-done-{}", phase.name());
        while !tokio::fs::try_exists(cli.signals.join(&done)).await? {
            if stopped(cli).await? {
                return Err("roam supervisor stopped the run".into());
            }
            for &role in roles {
                if let Ok(view) = client
                    .get_node_state(
                        &GetNodeStatePath {
                            network_id: network,
                            node_id: nodes[role.name()],
                        },
                        token,
                    )
                    .await
                {
                    write(
                        &cli.evidence.join(format!("state-{}.json", role.name())),
                        &view,
                    )
                    .await?;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    wait(cli, "roam-complete").await
}
