//! The real Server owner of a runtime row and its final report checks.
use super::*;

/// The row's real Server and how to restart it in place: the same database
/// and keys under a new process boot, as `dev.py restart server` does.
pub(super) struct Server<'a> {
    cli: &'a Cli,
    app: tokio::sync::Mutex<Option<App>>,
    routers: watch::Sender<axum::Router>,
    audit: crate::traversal::Audit,
    availability: crate::recovery::Availability,
    requests: crate::control_requests::Audit,
}

impl Server<'_> {
    async fn open(cli: &Cli) -> Result<App> {
        let mut settings = Settings::new(cli.state.join("server"));
        settings.netmap.validity = cli.netmap_validity.as_std();
        if matches!(
            cli.traversal,
            crate::TraversalMode::Nat | crate::TraversalMode::Roam
        ) {
            let controller = if cli.traversal == crate::TraversalMode::Roam {
                crate::roam::read::<crate::roam::Topology>(&cli.public.join("roam-topology.json"))
                    .await?
                    .controller
            } else {
                signals::underlay(cli).await?.controller
            };
            settings.stun_listen = vec!["0.0.0.0:3478".parse()?];
            settings.stun_servers = BTreeMap::from([(
                "fixture".into(),
                Endpoint {
                    host: controller.to_string(),
                    port: 3478.try_into()?,
                },
            )]);
        }
        Ok(App::open(settings).await?)
    }

    fn router(&self, app: &App) -> axum::Router {
        let mut router = meshway_server::http::router(app);
        if self.cli.traversal == crate::TraversalMode::Nat {
            router = router.layer(axum::middleware::from_fn_with_state(
                self.audit.clone(),
                crate::traversal::observe,
            ));
        }
        router = router.layer(axum::middleware::from_fn_with_state(
            self.availability.clone(),
            crate::recovery::gate,
        ));
        router = router.layer(axum::middleware::from_fn_with_state(
            self.requests.clone(),
            crate::control_requests::observe,
        ));
        if self.cli.traversal == crate::TraversalMode::Exit {
            // The hold precedes admission so closing the gate can drain already
            // admitted database work independently of deliberately held requests.
            router = router.layer(axum::middleware::from_fn_with_state(
                (self.cli.evidence.clone(), self.cli.signals.clone()),
                exit::observe_acquisition,
            ));
        }
        router
    }

    /// Stop the Server owner (every volatile State, fact and instance ends
    /// with it, and open connections are dropped) and open it again on the
    /// same database. Returns when the new owner serves, in Unix ms.
    pub(super) async fn restart(&self) -> Result<u64> {
        let mut app = self.app.lock().await;
        let old = app.take().ok_or("Server already shut down")?;
        old.stream_shutdown()();
        old.shutdown().await?;
        // The STUN listener's port is released by the old owner's shutdown.
        let mut attempts = 0;
        let new = loop {
            match Self::open(self.cli).await {
                Ok(new) => break new,
                Err(_) if attempts < 20 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                Err(error) => return Err(error),
            }
        };
        drop(self.routers.send_replace(self.router(&new)));
        *app = Some(new);
        crate::recovery::now_ms()
    }
}

pub(super) async fn run_server(cli: &Cli) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:8443").await?;
    let tls = meshway_packet_fixture::tls_server(&cli.public, &cli.private).await?;
    let app = Server::open(cli).await?;
    let (routers, _) = watch::channel(axum::Router::new());
    let server = Server {
        cli,
        app: tokio::sync::Mutex::new(None),
        routers,
        audit: crate::traversal::Audit::default(),
        availability: crate::recovery::Availability::new(),
        requests: crate::control_requests::Audit::new(),
    };
    drop(server.routers.send_replace(server.router(&app)));
    *server.app.lock().await = Some(app);
    let audit = server.audit.clone();
    let availability = server.availability.clone();
    let requests = server.requests.clone();
    let mut current = server.routers.subscribe();
    let (stop, mut stopped) = watch::channel(false);
    let accept = tokio::spawn(async move {
        let mut connections = JoinSet::new();
        let mut router = current.borrow_and_update().clone();
        let result: Result<()> = async {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    // A restarted Server: connections of the old owner end.
                    changed = current.changed() => {
                        if changed.is_err() { break; }
                        router = current.borrow_and_update().clone();
                        connections.abort_all();
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => {
                        match result {
                            Ok(result) => result?,
                            Err(error) if error.is_cancelled() => {}
                            Err(error) => return Err(error.into()),
                        }
                    }
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
        } else if cli.traversal == crate::TraversalMode::Roam {
            roam::coordinate(cli).await
        } else {
            coordinate(cli, &audit, &availability, &server).await
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
    let app = server.app.lock().await.take();
    if let Some(app) = &app {
        app.stream_shutdown()();
    }
    let server_result = accept.await;
    let store_result = match app {
        Some(app) => app.shutdown().await,
        None => Ok(()),
    };
    stop_result?;
    server_result??;
    store_result?;
    audit_result?;
    request_result?;
    result??;
    if matches!(
        cli.traversal,
        crate::TraversalMode::Exit | crate::TraversalMode::Roam
    ) {
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
