//! The real Server owner of a runtime row and its final report checks.
use super::*;

pub(super) async fn run_server(cli: &Cli) -> Result<()> {
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
