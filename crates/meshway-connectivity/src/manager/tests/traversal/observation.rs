use super::*;

struct PausedMetadata {
    inner: Arc<Gateway>,
    paused: AtomicBool,
    blocked: AtomicUsize,
    first_context_expires: Mutex<Option<std::time::Instant>>,
}

impl TraversalGateway for PausedMetadata {
    fn request(&self, operation: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(async move {
            if matches!(operation, Operation::Query(_)) && self.paused.load(Ordering::SeqCst) {
                self.blocked.fetch_add(1, Ordering::SeqCst);
                return std::future::pending().await;
            }
            let reply = self.inner.request(operation).await?;
            if let Reply::Query(meshway_control::connectivity::Outcome::Current(discovery)) = &reply
            {
                self.first_context_expires
                    .lock()
                    .unwrap()
                    .get_or_insert(discovery.context.expires_at());
            }
            Ok(reply)
        })
    }

    fn published(&self, candidate: &CandidateReference) -> bool {
        self.inner.published(candidate)
    }
}

#[tokio::test]
async fn live_discovery_context_refreshes_same_socket_without_another_metadata_rpc() {
    let StunService {
        address: stun,
        stop,
        service,
        ..
    } = StunService::start(false).await;
    let mut fixture = fixture::Fixture::builder().stun(stun).build().await;
    // Exercise observation independently of acquisition. There are no admitted
    // peer edges, but the actual unpublished UDP listener still needs STUN.
    fixture
        .configure(|cfg| {
            cfg.defaults.topology = meshway_configuration::TopologyDefault::Deny;
            cfg.topology.clear();
        })
        .await;
    fixture.refresh(0).await;
    let traffic = Arc::new(Traffic::default());
    let factory = NatFactory::new(&mut fixture, stun, traffic);
    let gateway = Arc::new(PausedMetadata {
        inner: Gateway::applied(fixture.controls.remove(0), None).await,
        paused: AtomicBool::new(false),
        blocked: AtomicUsize::new(0),
        first_context_expires: Mutex::new(None),
    });
    let mut manager = fixture
        .manager(0, factory)
        .traversal(gateway.clone(), Sequence::MIN.checked_next().unwrap())
        .start()
        .await;
    let original = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let socket = listener_socket(&manager);
            if !socket.endpoints.is_empty() {
                break socket;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    gateway.paused.store(true, Ordering::SeqCst);
    let refreshed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let socket = listener_socket(&manager);
            if socket.endpoints.iter().any(|candidate| {
                candidate.observed_at > original.endpoints[0].observed_at
                    && candidate.expires_at > original.endpoints[0].expires_at
            }) {
                break socket;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    let renewed_before_expiry = if refreshed.is_ok() {
        // Reuse must not defer necessary metadata renewal until the original
        // context has already expired, creating a periodic observation gap.
        let expires = gateway.first_context_expires.lock().unwrap().unwrap();
        let queries = gateway.inner.queries.load(Ordering::SeqCst);
        gateway.paused.store(false, Ordering::SeqCst);
        tokio::time::timeout_at((expires - Duration::from_secs(1)).into(), async {
            while gateway.inner.queries.load(Ordering::SeqCst) == queries {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .is_ok()
    } else {
        false
    };
    // Always stop and join the real socket/service owners, including the red
    // case where an unnecessary metadata request blocked a usable context.
    manager.shutdown().await.unwrap();
    drop(manager);
    stop.cancel();
    service.await.unwrap();
    let blocked = gateway.blocked.load(Ordering::SeqCst);
    let gateway = Arc::try_unwrap(gateway).ok().expect("joined gateway");
    Arc::try_unwrap(gateway.inner)
        .ok()
        .expect("joined control")
        .control
        .into_inner()
        .shutdown()
        .await
        .unwrap();
    fixture.close().await;
    let refreshed = refreshed.expect("live context must refresh STUN without waiting for metadata");
    assert_eq!(refreshed.reference, original.reference);
    assert_eq!(refreshed.bound_address, original.bound_address);
    assert_eq!(blocked, 0);
    assert!(
        renewed_before_expiry,
        "renew metadata before the original context deadline"
    );
}
