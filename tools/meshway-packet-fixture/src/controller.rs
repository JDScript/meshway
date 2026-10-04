use crate::{
    Cli, IO, ORIGIN, Result, STARTUP, request,
    signals::{Role, Signal, Signals},
    tls,
};
use meshway_configuration::{AccessRule, MeshRoute, NetworkConfigurationInput};
use meshway_control_client::{
    ApplyConfigurationPath, ApproveDeviceAuthorizationPath, Client, GetConfigurationSourcePath,
    GetConfigurationSourceQuery, GetDeviceAuthorizationPath, ListNodesPath,
};
use meshway_protocol::{
    ApproveEnrollmentRequest, ConfigurationRequest, CreateNetworkRequest, RegisterRequest,
    WireSecret,
};
use meshway_server::{App, Settings};
use std::{collections::BTreeMap, time::Duration};
use tokio::{net::TcpListener, sync::watch, task::JoinSet};

pub async fn run(cli: &Cli) -> Result<()> {
    let app = App::open(Settings::new(cli.state.join("server"))).await?;
    let listener = TcpListener::bind("0.0.0.0:8443").await?;
    let server = tls::server(cli).await?;
    let router = meshway_server::http::router(&app);
    let (stop, mut stopped) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                _ = stopped.changed() => break,
                Some(_) = connections.join_next(), if !connections.is_empty() => {},
                accepted = listener.accept(), if connections.len() < 64 => {
                    let (stream, _) = accepted?;
                    let server = server.clone();
                    let router = router.clone();
                    connections.spawn(async move {
                        if let Ok(stream) = server.accept(stream).await {
                            let service = hyper_util::service::TowerToHyperService::new(router);
                            let _ = tokio::time::timeout(IO, hyper::server::conn::http1::Builder::new()
                                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)).await;
                        }
                    });
                }
            }
        }
        connections.abort_all();
        while connections.join_next().await.is_some() {}
        Ok::<(), crate::Error>(())
    });
    tracing::info!(
        phase = "controller-listening",
        "fixture controller HTTPS ready"
    );
    let result = tokio::time::timeout(Duration::from_secs(150), coordinate(cli)).await;
    let _ = stop.send(true);
    app.stream_shutdown()();
    tokio::time::timeout(IO, task).await???;
    tokio::time::timeout(IO, app.shutdown()).await??;
    result?
}

async fn coordinate(cli: &Cli) -> Result<()> {
    let signals = Signals::new(&cli.signals);
    let client = Client::new(ORIGIN, tls::client(cli).await?, 16 * 1024 * 1024)?;
    // The owner credential is created and retained only in this controller actor.
    let session = client
        .register(&RegisterRequest {
            email: "isolated-packet-owner@example.test".into(),
            password: WireSecret::new(format!("fixture-owner-{}", request()?)),
        })
        .await?;
    let network = client
        .create_network(
            &CreateNetworkRequest {
                request_id: request()?,
                name: "Isolated packet conformance".into(),
            },
            session.token.expose(),
        )
        .await?
        .network
        .id;
    let mut enrollments = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Enrollment {
            role: claimed,
            request,
        } = signals.wait(&format!("enrollment-{}", role.name())).await?
        else {
            return Err("invalid enrollment signal".into());
        };
        if claimed != role || enrollments.values().any(|old| *old == request) {
            return Err("duplicate enrollment signal".into());
        }
        let actual = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: request,
                },
                session.token.expose(),
            )
            .await?;
        if actual.request.hostname != role.name() {
            return Err("enrollment hostname mismatch".into());
        }
        client
            .approve_device_authorization(
                &ApproveDeviceAuthorizationPath {
                    enrollment_request_id: request,
                },
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
                session.token.expose(),
            )
            .await?;
        enrollments.insert(role, request);
    }
    let mut nodes = BTreeMap::new();
    for role in Role::ALL {
        let Signal::Enrolled {
            role: claimed,
            node,
        } = signals.wait(&format!("enrolled-{}", role.name())).await?
        else {
            return Err("invalid enrolled signal".into());
        };
        if claimed != role || nodes.values().any(|old| *old == node) {
            return Err("duplicate node signal".into());
        }
        nodes.insert(role, node);
    }
    let inventory = client
        .list_nodes(
            &ListNodesPath {
                network_id: network,
            },
            session.token.expose(),
        )
        .await?;
    if inventory.items.len() != 3 {
        return Err("unexpected fixture inventory".into());
    }
    for role in Role::ALL {
        let node = inventory
            .items
            .iter()
            .find(|node| node.id == nodes[&role])
            .ok_or("node not registered")?;
        let approval = client
            .get_device_authorization(
                &GetDeviceAuthorizationPath {
                    enrollment_request_id: enrollments[&role],
                },
                session.token.expose(),
            )
            .await?;
        if node.hostname != role.name()
            || node.network_id != network
            || node.keys.control.public_key != approval.request.control_key
        {
            return Err("registered enrollment binding mismatch".into());
        }
    }
    let source = client
        .get_configuration_source(
            &GetConfigurationSourcePath {
                network_id: network,
            },
            &GetConfigurationSourceQuery::default(),
            session.token.expose(),
        )
        .await?;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    for role in Role::ALL {
        let node = input
            .nodes
            .get_mut(&nodes[&role])
            .ok_or("missing node configuration")?;
        node.listeners = serde_json::from_value(serde_json::json!({"tcp": {
            "type":"native_tcp", "listen":"0.0.0.0", "listenPort":35000,
            "endpoints":[{"host":format!("meshway-packet-{}", role.name()), "port":35000}]
        }}))?;
    }
    input.access = vec![AccessRule {
        remark: String::new(),
        src: vec![format!("node:{}", nodes[&Role::A])],
        dst: vec![format!("node:{}", nodes[&Role::B])],
        ip: vec!["udp:42424".into()],
        bidirectional: false,
    }];
    input.routing.mesh = vec![MeshRoute {
        remark: String::new(),
        src: vec![format!("node:{}", nodes[&Role::A])],
        dst: vec![format!("node:{}", nodes[&Role::B])],
        via: vec![format!("node:{}", nodes[&Role::R])],
    }];
    client
        .apply_configuration(
            &ApplyConfigurationPath {
                network_id: network,
            },
            &ConfigurationRequest::Document {
                expected_revision: source.revision,
                request_id: request()?,
                source_jsonc: WireSecret::new(serde_json::to_string(&input)?),
            },
            session.token.expose(),
        )
        .await?;
    signals.write("go", Signal::Go).await?;
    tracing::info!(
        phase = "configuration-committed",
        "real enrolled nodes configured"
    );
    if !matches!(signals.wait("done").await?, Signal::Done) {
        return Err("invalid completion".into());
    }
    for role in Role::ALL {
        let report =
            tokio::time::timeout(STARTUP, signals.wait(&format!("report-{}", role.name())))
                .await??;
        let Signal::Report {
            role: claimed,
            forwarded,
            delivered,
            tun_removed,
        } = report
        else {
            return Err("invalid report".into());
        };
        if claimed != role || forwarded < 2 {
            return Err("incomplete wire evidence".into());
        }
        match role {
            Role::R if delivered.is_none() && tun_removed.is_none() => {}
            Role::A | Role::B
                if delivered.is_some_and(|count| count >= 2) && tun_removed == Some(true) => {}
            _ => return Err("incomplete endpoint evidence".into()),
        }
        tracing::info!(
            role = role.name(),
            forwarded,
            delivered,
            tun_removed,
            phase = "validated",
            "packet actor conformance report"
        );
    }
    Ok(())
}
