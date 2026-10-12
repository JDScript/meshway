//! Exit configuration is submitted through the generated management client.
use super::*;
use crate::exit::{Role as ExitRole, Topology, read, ready, wait, write};
use meshway_configuration::{ProfileAssignment, RouteAction, RoutingProfile};

/// Direct ranges of the split resolver's rule set: a country-IP-set scale.
const DIRECT_RANGES: u32 = 7_000;

pub(super) async fn observe_acquisition(
    axum::extract::State((evidence, signals)): axum::extract::State<(
        std::path::PathBuf,
        std::path::PathBuf,
    )>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if request.uri().path().starts_with("/agent-api/v1/")
        && tokio::fs::try_exists(signals.join("exit-http-hold"))
            .await
            .unwrap_or(false)
        && !tokio::fs::try_exists(signals.join("exit-http-open"))
            .await
            .unwrap_or(false)
    {
        if tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(signals.join("exit-http-held"))
            .await
            .is_ok()
        {
            tracing::info!(
                phase = "exit-http-held",
                path = request.uri().path(),
                "real Node HTTP request held before admission"
            );
        }
        let held = tokio::time::timeout(Duration::from_secs(180), async {
            loop {
                if tokio::fs::try_exists(signals.join("exit-http-open")).await?
                    || tokio::fs::try_exists(signals.join("exit-stop")).await?
                {
                    break Ok::<(), std::io::Error>(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if !matches!(held, Ok(Ok(()))) {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    if request.uri().path() == "/agent-api/v1/sync" {
        // An outage also ends sync long polls already in flight, like a real
        // partition: no Node receives another lease after the hold begins.
        let (hold, open) = (
            signals.join("exit-http-hold"),
            signals.join("exit-http-open"),
        );
        return tokio::select! {
            response = next.run(request) => response,
            () = async {
                while !tokio::fs::try_exists(&hold).await.unwrap_or(false)
                    || tokio::fs::try_exists(&open).await.unwrap_or(true)
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            } => axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
        };
    }
    if request.uri().path() == "/agent-api/v1/dns-associations" {
        // Every accepted association task completion is one Server round
        // trip of the domain exit stage; the count is its evidence.
        static ACCEPTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let response = next.run(request).await;
        if response.status().is_success() {
            let count = ACCEPTED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if write(&evidence.join("association-reports.json"), &count)
                .await
                .is_err()
            {
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
        return response;
    }
    if request.uri().path() != "/agent-api/v1/instances" {
        return next.run(request).await;
    }
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 64 * 1024).await else {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(acquire) = serde_json::from_slice::<meshway_protocol::AcquireInstanceRequest>(&bytes)
    else {
        return axum::http::StatusCode::BAD_REQUEST.into_response();
    };
    let response = next
        .run(axum::extract::Request::from_parts(parts, bytes.into()))
        .await;
    if response.status().is_success()
        && write(
            &evidence.join(format!("acquire-{}.json", acquire.instance_id)),
            &acquire,
        )
        .await
        .is_err()
    {
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    response
}

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
    let mut input = original.configuration.clone();
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
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
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
        let status: meshway_protocol::ipc::Status = read(&cli.signals.join(name)).await?;
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
    input = original.configuration.clone();
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
            keepalive: false,
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
    // Ordinary names go to E's resolver before resolution; MagicDNS stays
    // local and the useWithExit split resolver keeps its own direct route.
    let split = topology.addresses["split"]["resolver"];
    input.dns.magic_dns = true;
    input.dns.servers = BTreeMap::from([(
        "split".into(),
        meshway_configuration::DnsServer {
            transport: meshway_configuration::DnsTransport::Udp,
            server: split.into(),
            server_port: 53.try_into()?,
            use_with_exit: true,
        },
    )]);
    input.dns.routes = BTreeMap::from([(
        crate::exit::SPLIT_ZONE.trim_end_matches('.').into(),
        vec!["split".into()],
    )]);
    // The split resolver is direct through a rule set of ~7,000 ranges (a
    // country-IP-set scale, built offline): A computes and installs the whole
    // partition from the artifact, 7,000 direct holes in the exit capture.
    let mut direct: Vec<String> = (0..DIRECT_RANGES)
        .map(|i| format!("{}/20", std::net::Ipv4Addr::from(0x0c00_0000 + i * 0x2000)))
        .collect();
    direct.push(format!("{split}/32"));
    input.rule_sets.insert(
        "exit-direct".into(),
        serde_json::from_value(serde_json::json!({"type": "inline", "ipCIDR": direct}))?,
    );
    // The exit LAN is routed by a rule naming a rule set: A's egress
    // partition and every A decision use its artifact, fetched by digest. A
    // missing artifact would fail every exit flow closed (this rule precedes
    // the fallback), so the row's exit traffic proves the artifact is held.
    input.rule_sets.insert(
        "exit-lan".into(),
        serde_json::from_value(serde_json::json!({
            "type": "inline", "ipCIDR": [topology.lan_cidr.clone()]
        }))?,
    );
    input.routing.egress.profiles.insert(
        "exit".into(),
        RoutingProfile {
            rules: vec![
                meshway_configuration::EgressRule {
                    remark: "useWithExit split resolver among 7,000 direct ranges".into(),
                    matcher: meshway_configuration::RouteMatch {
                        rule_sets: vec!["exit-direct".into()],
                        ..Default::default()
                    },
                    action: meshway_configuration::RuleAction::Direct,
                },
                meshway_configuration::EgressRule {
                    remark: "exit LAN through a rule set".into(),
                    matcher: meshway_configuration::RouteMatch {
                        rule_sets: vec!["exit-lan".into()],
                        ..Default::default()
                    },
                    action: meshway_configuration::RuleAction::Exit {
                        nodes: vec![format!("node:{}", nodes["e"])],
                        via: vec![format!("node:{}", nodes["r"])],
                    },
                },
            ],
            fallback: RouteAction::Exit {
                nodes: vec![format!("node:{}", nodes["e"])],
                via: vec![format!("node:{}", nodes["r"])],
            },
        },
    );
    input.routing.egress.assignments = vec![ProfileAssignment {
        target: vec![format!("node:{}", nodes["a"])],
        allowed: vec![
            "exit".into(),
            "mesh".into(),
            format!("global:{}", nodes["e"]),
        ],
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
    // Domain exit stage: names under NAME_ZONE exit through E via R by a
    // name rule, everything else of A is direct; Access grants the names.
    let original_input = input.clone();
    let mut named = input.clone();
    let name_zone = crate::exit::NAME_ZONE.trim_end_matches('.').to_owned();
    for rule in &mut named.access {
        for destination in &mut rule.dst {
            if destination == "autogroup:internet" {
                *destination = format!("domain:{name_zone}");
            }
        }
    }
    let profile = named
        .routing
        .egress
        .profiles
        .get_mut("exit")
        .ok_or("missing exit profile")?;
    profile.rules.push(meshway_configuration::EgressRule {
        remark: "names through E by a trusted association".into(),
        matcher: meshway_configuration::RouteMatch {
            domain_suffix: vec![name_zone.clone()],
            ..Default::default()
        },
        action: meshway_configuration::RuleAction::Exit {
            nodes: vec![format!("node:{}", nodes["e"])],
            via: vec![format!("node:{}", nodes["r"])],
        },
    });
    profile.fallback = RouteAction::Direct;
    let mut revoked = named.clone();
    for rule in &mut revoked.access {
        rule.dst
            .retain(|destination| !destination.starts_with("domain:"));
    }
    let mut name_stage = 0;
    let mut outage = false;
    let mut reopened = false;
    let mut withdrawn = false;
    let mut restored = false;
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
        for (step, signal, done, target) in [
            (
                0,
                "exit-name-configure",
                "exit-name-configured.json",
                &named,
            ),
            (1, "exit-name-revoke", "exit-name-revoked.json", &revoked),
            (
                2,
                "exit-name-restore",
                "exit-name-restored.json",
                &original_input,
            ),
        ] {
            if name_stage == step && tokio::fs::try_exists(cli.signals.join(signal)).await? {
                let current = source(&client, network, token).await?;
                let revision = apply(&client, network, token, current.revision, target).await?;
                write(&cli.signals.join(done), &revision).await?;
                name_stage += 1;
            }
        }
        if !outage && tokio::fs::try_exists(cli.signals.join("exit-http-close")).await? {
            ready(cli, "exit-http-hold").await?;
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
        if outage && !reopened && tokio::fs::try_exists(cli.signals.join("exit-http-open")).await? {
            // A held handler (or the next retry if its peer disconnected) first
            // reaches the closed admission gate. Retain the gate's real rejected
            // request proof before reopening it.
            let opened = tokio::time::timeout(STARTUP, async {
                loop {
                    if let Ok(opened) = availability.open() {
                        return opened;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            })
            .await?;
            write(&cli.signals.join("exit-http-opened.json"), &opened).await?;
            reopened = true;
        }
        if withdrawn && !restored && tokio::fs::try_exists(cli.signals.join("exit-restore")).await?
        {
            let current = source(&client, network, token).await?;
            input
                .nodes
                .get_mut(&nodes["e"])
                .ok_or("missing E")?
                .exit_enabled = true;
            let revision = apply(&client, network, token, current.revision, &input).await?;
            write(&cli.signals.join("exit-restored.json"), &revision).await?;
            restored = true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn exit_http_hold_precedes_closed_admission_and_releases_a_real_request() {
        let directory =
            std::env::temp_dir().join(format!("meshway-exit-hold-{}", request().unwrap()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        let result = exercise(&directory).await;
        std::fs::remove_dir_all(directory).unwrap();
        result.unwrap();
    }

    async fn exercise(directory: &std::path::Path) -> Result<()> {
        let availability = crate::recovery::Availability::new();
        availability.close().await?;
        write(&directory.join("exit-http-hold"), &true).await?;
        let router = axum::Router::new()
            .route(
                "/agent-api/v1/test",
                axum::routing::get(|| async { "admitted" }),
            )
            .layer(axum::middleware::from_fn_with_state(
                availability.clone(),
                crate::recovery::gate,
            ))
            .layer(axum::middleware::from_fn_with_state(
                (directory.to_path_buf(), directory.to_path_buf()),
                observe_acquisition,
            ));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        let result = async {
            let mut client = tokio::net::TcpStream::connect(address).await?;
            client.write_all(b"GET /agent-api/v1/test HTTP/1.1\r\nHost: fixture\r\nConnection: close\r\n\r\n").await?;
            tokio::time::timeout(IO, async {
                while !tokio::fs::try_exists(directory.join("exit-http-held")).await? {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                Ok::<(), std::io::Error>(())
            }).await??;
            let mut prefix = [0; 12];
            if tokio::time::timeout(Duration::from_millis(50), client.read_exact(&mut prefix)).await.is_ok() {
                return Err("HTTP request was rejected instead of held".into());
            }
            write(&directory.join("exit-http-open"), &true).await?;
            tokio::time::timeout(IO, client.read_exact(&mut prefix)).await??;
            if &prefix != b"HTTP/1.1 503" { return Err("held request bypassed closed admission".into()); }
            if availability.open()?.1 != 1 { return Err("missing real rejected-request receipt".into()); }
            Ok::<(), crate::Error>(())
        }.await;
        // Also release held handlers on a failed assertion before joining.
        write(&directory.join("exit-stop"), &true).await?;
        let _ = stop.send(());
        server.await??;
        result
    }
}
