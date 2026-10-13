use super::*;

#[tokio::test]
async fn resource_scan_shutdown_releases_its_waiter_before_accepted_sql_drains() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let app = App::open(Settings::new(path.clone())).await.unwrap();
    let cache_owners = Arc::strong_count(&app.committer.cache);
    let handle = app.executor.handle();
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let committed = Arc::new(AtomicBool::new(false));
    let completed = committed.clone();
    let blocker = tokio::spawn(async move {
        handle.execute(move |db| {
            let tx = db.transaction().unwrap();
            tx.execute_batch("CREATE TEMP TABLE accepted_resource_work(value INTEGER); INSERT INTO accepted_resource_work VALUES(1)").unwrap();
            entered.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(10)).unwrap();
            tx.commit().unwrap();
            completed.store(true, Ordering::Release);
        }).await.unwrap();
    });
    started.await.unwrap();
    // The scheduler clones its provider before awaiting the serial owner. The
    // extra cache reference proves it has entered a scan (or the artifact
    // collection it starts with) behind the blocked job.
    tokio::time::timeout(Duration::from_secs(3), async {
        while Arc::strong_count(&app.committer.cache) == cache_owners {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), app.resource_scheduler.shutdown())
        .await
        .unwrap();
    assert!(!committed.load(Ordering::Acquire));
    assert!(matches!(
        App::open(Settings::new(path.clone())).await,
        Err(AppError::AlreadyRunning)
    ));
    release.send(()).unwrap();
    blocker.await.unwrap();
    assert!(committed.load(Ordering::Acquire));
    app.shutdown_hooks.close();
    app.executor.shutdown().await.unwrap();
    App::open(Settings::new(path))
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn startup_scheduler_updates_backs_off_persists_and_cancels_its_tls_socket() {
    // C: real scheduler MIN_INTERVAL_MS (1000) and persisted failure backoff
    // must pass across restart; view predicates/Notify observe actual work.
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("scheduler@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
    let mut definition = remote.definition();
    if let RuleSet::Remote {
        update_interval, ..
    } = &mut definition
    {
        *update_interval = "1ms".parse().unwrap();
    }
    input.rule_sets.insert("remote".into(), definition);
    s.apply(token, network, current.revision, &input).await;
    let initial = s.view(token, network).await;
    let first = initial.artifact.unwrap().digest;
    s.close().await;
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    // No explicit refresh or retained administrator session is passed to the
    // worker: ordinary App startup resumes from persisted check deadlines.
    let s = Server::open(&path, remote.clone()).await;
    let updated = wait_view(&s, token, network, |v| {
        v.artifact.as_ref().is_some_and(|a| a.digest != first)
    })
    .await;
    let counts = resource_counts(&s, network).await;
    remote.set(200, "payload:\n - 8.8.8.9/24\n - 8.8.8.0/24\n");
    let unchanged = wait_view(&s, token, network, |v| v.checked_at > updated.checked_at).await;
    assert_eq!(unchanged.artifact, updated.artifact);
    assert_eq!(resource_counts(&s, network).await, counts);
    remote.set(503, "unavailable");
    let failed = wait_view(&s, token, network, |v| {
        v.problem == Some(ResourceProblem::DownloadFailed)
    })
    .await;
    s.close().await;
    let s = Server::open(&path, remote.clone()).await;
    let retried = wait_view(&s, token, network, |v| v.checked_at > failed.checked_at).await;
    // Inspect the actual retry's timestamp. Restart and inspection may finish
    // after backoff on a busy host; neither operation is required to win it.
    let failed_at = failed.checked_at.unwrap();
    let first_retry = *remote
        .request_times
        .lock()
        .unwrap()
        .iter()
        .find(|at| at.timestamp_millis() > failed_at.timestamp_millis())
        .expect("actual first HTTP retry, even if the view skipped a check");
    assert!(
        first_retry.timestamp_millis() as u64
            >= crate::resources::scheduler::due_at(
                Some(failed_at.timestamp_millis() as u64),
                crate::resources::scheduler::MIN_INTERVAL_MS,
                1,
            )
    );
    assert_eq!(retried.artifact, failed.artifact);
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    s.refresh(token, network).await;
    remote.hold_body.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), remote.body_entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), s.close())
        .await
        .expect("shutdown joins cancellation, not download timeout");
    tokio::time::timeout(Duration::from_secs(2), async {
        while !remote.peer_closed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("TLS socket closed by cancellation");
    remote.hold_body.store(false, Ordering::SeqCst);
    let s = Server::open(&path, remote.clone()).await;
    remote.peer_closed.store(false, Ordering::SeqCst);
    remote.hold_body.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(3), remote.body_entered.notified())
        .await
        .unwrap();
    let Server {
        app, stop, task, ..
    } = s;
    stop.send(()).unwrap();
    task.await.unwrap();
    drop(app);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !remote.peer_closed.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("App drop cancels owned network work");
    remote.hold_body.store(false, Ordering::SeqCst);
}

#[tokio::test]
async fn download_races_recheck_source_role_and_scope_and_do_not_hold_sqlite() {
    use meshway_protocol::management::*;
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("race-owner@example.com").await;
    let admin = s.register("race-admin@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let outside = s.create(admin.token.expose()).await.id;
    let link: JoinLinkCreated = json(
        s.client
            .post(s.network(network, "join-links"))
            .bearer_auth(token)
            .json(&CreateJoinLinkRequest {
                request_id: rid(),
                role: MembershipRole::Admin,
                expires_in_seconds: 600,
            }),
    )
    .await;
    let JoinLinkCreated::Available { secret, .. } = link else {
        panic!("new link");
    };
    let joined: JoinLinkRedeemed = json(
        s.client
            .post(format!("{}/api/v1/join-links/redeem", s.url))
            .bearer_auth(admin.token.expose())
            .json(&JoinLinkCredentialRequest {
                token: WireSecret::new(secret.expose().into()),
            }),
    )
    .await;
    let source = s.source(token, network).await;
    let mut input = source.configuration.clone();
    input.rule_sets.insert("remote".into(), remote.definition());
    s.apply(token, network, source.revision, &input).await;
    let first = s.refresh(token, network).await.artifact.unwrap().digest;
    // Concurrent callers join one real TLS transfer; only the first final
    // checker can update freshness, the stale waiter cannot overwrite it.
    remote.pause();
    let count = remote.requests.load(Ordering::SeqCst);
    let one = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id: rid() })
            .send(),
    );
    remote.entered.notified().await;
    let two = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id: rid() })
            .send(),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    remote.resume.notify_one();
    let mut statuses = [
        one.await.unwrap().unwrap().status().as_u16(),
        two.await.unwrap().unwrap().status().as_u16(),
    ];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(remote.requests.load(Ordering::SeqCst), count + 1);
    let cross = s
        .client
        .get(s.network(outside, "rule-sets/remote"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), StatusCode::NOT_FOUND);
    remote.pause();
    let refresh = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(token)
            .json(&RefreshRuleSetRequest { request_id: rid() })
            .send(),
    );
    remote.entered.notified().await;
    // This normal configuration request completes while DNS/download is paused.
    let source = s.source(token, network).await;
    let mut input = source.configuration.clone();
    if let RuleSet::Remote { url, .. } = input.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/changed");
    }
    let changed = s.apply(token, network, source.revision, &input).await;
    remote.resume.notify_one();
    assert_eq!(
        refresh.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    let pending = s.view(token, network).await;
    assert_eq!(pending.configuration_revision, changed.revision);
    assert_eq!(pending.artifact.unwrap().digest, first);
    assert!(pending.problem.is_none());
    assert_eq!(
        s.refresh(token, network).await.artifact.unwrap().digest,
        first
    );

    remote.pause();
    let refresh = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(admin.token.expose())
            .json(&RefreshRuleSetRequest { request_id: rid() })
            .send(),
    );
    remote.entered.notified().await;
    let response = s
        .client
        .post(s.network(network, &format!("members/{}/role", joined.membership_id)))
        .bearer_auth(token)
        .json(&ChangeMemberRoleRequest {
            expected_version: Version::MIN,
            role: MembershipRole::Member,
        })
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    remote.resume.notify_one();
    assert_eq!(
        refresh.await.unwrap().unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let forbidden = s
        .client
        .get(s.network(network, "rule-sets"))
        .bearer_auth(admin.token.expose())
        .send()
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    assert!(
        !forbidden
            .text()
            .await
            .unwrap()
            .contains("rules.meshway.example")
    );

    remote.private_dns.store(true, Ordering::SeqCst);
    assert_eq!(
        s.refresh(token, network).await.problem,
        Some(ResourceProblem::InvalidSource)
    );
    remote.private_dns.store(false, Ordering::SeqCst);
    remote.set(
        200,
        &"x".repeat(meshway_rules::convert::MAX_SOURCE_BYTES + 1),
    );
    assert_eq!(
        s.refresh(token, network).await.problem,
        Some(ResourceProblem::TooLarge)
    );
    assert_eq!(s.view(token, network).await.artifact.unwrap().digest, first);
    // The production transaction is accepted before cancelling its awaiting
    // task. Its publication and resource journal must still commit together.
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let service = s
        .app
        .resource_service(CommitGate {
            provider: crate::resources::provider::Provider {
                sealer: s.app.sealer(),
                cache: s.app.committer.cache.clone(),
            },
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(gate),
        })
        .with_connector(remote.clone());
    let identity = crate::state::service::ReadIdentity::user_session(token, network).unwrap();
    let sequence = s.app.state_runtime.lock().unwrap().hub.events.sequence();
    let accepted = tokio::spawn(async move {
        service
            .refresh_rule_set(identity, "remote".into(), rid())
            .await
    });
    started.await.unwrap();
    accepted.abort();
    assert!(accepted.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    let committed = s.view(token, network).await;
    assert!(committed.problem.is_none());
    let committed_id = committed.artifact.unwrap().digest;
    assert_ne!(committed_id, first);
    assert!(
        s.app
            .state_runtime
            .lock()
            .unwrap()
            .hub
            .events
            .after(sequence, network)
            .unwrap()
            .iter()
            .any(|entry| matches!(entry.change, crate::effect::Effect::Resource { .. }))
    );
    remote.set(503, "unavailable");
    assert_eq!(
        s.refresh(token, network).await.problem,
        Some(ResourceProblem::DownloadFailed)
    );
    s.close().await;
    let s = Server::open(&path, remote).await;
    assert_eq!(
        s.view(token, network).await.problem,
        Some(ResourceProblem::DownloadFailed)
    );
    assert_eq!(
        s.view(token, network).await.artifact.unwrap().digest,
        committed_id
    );
    // An unrelated edit keeps the failed check with the unchanged binding:
    // the new version does not pretend the set was never checked.
    let failed = s.view(token, network).await;
    let current = s.source(token, network).await;
    let mut input = current.configuration.clone();
    input.tags_owners.insert("tag:later".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let carried = s.view(token, network).await;
    assert_eq!(carried.problem, Some(ResourceProblem::DownloadFailed));
    assert_eq!(carried.checked_at, failed.checked_at);
    assert_eq!(carried.last_success_at, failed.last_success_at);
    assert_eq!(carried.artifact.unwrap().digest, committed_id);
    s.close().await;
}
