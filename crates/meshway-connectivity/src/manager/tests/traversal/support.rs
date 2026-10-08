use super::*;

pub(super) fn public(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if address.is_ipv4() {
            "198.51.100.1"
        } else {
            "2001:db8::1"
        }
        .parse()
        .unwrap(),
        address.port(),
    )
}
pub(super) fn private(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(
        if address.is_ipv4() {
            "127.0.0.1"
        } else {
            "::1"
        }
        .parse()
        .unwrap(),
        address.port(),
    )
}

pub(super) fn listener_socket(manager: &Manager) -> SocketFact {
    manager
        .snapshot()
        .unwrap()
        .sockets
        .into_iter()
        .find(|socket| socket.reference.resource == (ResourceRef::Listener { key: "udp".into() }))
        .unwrap()
}

pub(super) async fn inject_adjacency(
    sender: &AsyncMutex<Sender>,
    remote: SocketAddr,
    bytes: Vec<u8>,
) {
    let mut frames = std::collections::VecDeque::from([Frame::new(bytes, Some(remote))]);
    let admitted = sender
        .lock()
        .await
        .try_send(&mut frames, Default::default());
    assert_eq!(admitted.accepted, 1);
    let completed =
        tokio::time::timeout(Duration::from_secs(2), admitted.completion.unwrap().wait())
            .await
            .unwrap()
            .unwrap();
    assert!(matches!(
        completed.frames[0].outcome,
        meshway_transport::SendOutcome::LocalComplete
    ));
}

#[derive(Default)]
pub(super) struct Traffic {
    pub(super) timeline: Option<Arc<AcquisitionTimeline>>,
    pub(super) sent: Mutex<Vec<(SocketAddr, SocketAddr, Vec<u8>)>>,
    pub(super) block_traversal: AtomicBool,
    pub(super) blocked: Mutex<Option<Blocked>>,
    pub(super) authorized: Mutex<BTreeMap<(SocketAddr, [u8; 32]), AttemptScope>>,
    pub(super) channels: Mutex<Vec<ChannelAuthorization>>,
    pub(super) waker: AtomicWaker,
    pub(super) block_data: AtomicBool,
    pub(super) data_blocked: AtomicBool,
    pub(super) data_writes: AtomicUsize,
    pub(super) opened: AtomicUsize,
    pub(super) dropped: AtomicUsize,
}

pub(super) struct AcquisitionTimeline {
    pub(super) started: std::time::Instant,
    pub(super) events: Mutex<Vec<String>>,
}
impl AcquisitionTimeline {
    pub(super) fn new() -> Arc<Self> {
        Arc::new(Self {
            started: std::time::Instant::now(),
            events: Mutex::new(Vec::new()),
        })
    }
    pub(super) fn record(&self, event: std::fmt::Arguments<'_>) {
        let mut events = self.events.lock().unwrap();
        if events.len() < 1024 {
            events.push(format!(
                "{}ms utc={} {event}",
                self.started.elapsed().as_millis(),
                chrono::Utc::now().timestamp_millis()
            ));
        }
    }
    pub(super) fn packet(
        &self,
        direction: &str,
        local: SocketAddr,
        remote: SocketAddr,
        bytes: &[u8],
    ) {
        if bytes.starts_with(b"MWTRV1\0\0") && bytes.len() >= 41 {
            self.record(format_args!(
                "{direction} local={local} remote={remote} kind={} digest={:02x?}",
                bytes[40],
                &bytes[8..40]
            ));
        }
    }
    pub(super) fn dump(&self) {
        for event in self.events.lock().unwrap().iter() {
            eprintln!("ACQUISITION {event}");
        }
    }
}
pub(super) struct ChannelAuthorization {
    pub(super) local: SocketAddr,
    pub(super) attempt: AttemptScope,
    pub(super) lifetime: Arc<dyn meshway_transport::SendLifetime>,
}
#[derive(Clone)]
pub(super) struct Blocked {
    pub(super) local: SocketAddr,
    pub(super) remote: SocketAddr,
    pub(super) digest: [u8; 32],
    pub(super) waker: std::task::Waker,
}

pub(super) struct Gateway {
    pub(super) control: AsyncMutex<Control>,
    pub(super) queries: AtomicUsize,
    pub(super) acknowledged: Mutex<Vec<CandidateReference>>,
    pub(super) timeline: Option<Arc<AcquisitionTimeline>>,
}

pub(super) struct OfflineGateway;
impl TraversalGateway for OfflineGateway {
    fn request(&self, _: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(std::future::pending())
    }
    fn published(&self, _: &CandidateReference) -> bool {
        false
    }
}

impl TraversalGateway for Gateway {
    fn request(&self, operation: Operation) -> BoxFuture<'_, Result<Reply, Error>> {
        Box::pin(async move {
            let queued = std::time::Instant::now();
            let mut control = self.control.lock().await;
            let started = std::time::Instant::now();
            let snapshot = self
                .timeline
                .as_ref()
                .and_then(|_| control.execution_snapshot().ok());
            let node = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.configuration().ok())
                .map(|cfg| cfg.node_id.to_string())
                .unwrap_or_default();
            if let (Some(timeline), Some(snapshot)) = (&self.timeline, &snapshot) {
                timeline.record(format_args!("rpc-start node={node} queue_ms={} instance={:?} publication={:?} execution_expires={} operation={}", queued.elapsed().as_millis(), snapshot.instance(), snapshot.reference().execution_publication, snapshot.expires_at_ms(), match &operation { Operation::Query(_) => "query", Operation::Peers { .. } => "peers", Operation::Prepare { .. } => "prepare", Operation::Accept(_) => "accept", Operation::Cancel(_) => "cancel" }));
                if let Operation::Accept(offer) | Operation::Cancel(offer) = &operation {
                    timeline.record(format_args!(
                        "original node={node} offer={:?} mono_remaining_ms={} current={:?}",
                        offer.offer(),
                        offer
                            .expires_at()
                            .saturating_duration_since(started)
                            .as_millis(),
                        offer.ensure_current()
                    ));
                }
            }
            let result: Result<Reply, meshway_control::Error> = async {
                Ok(match operation {
                    Operation::Query(snapshot) => {
                        self.queries.fetch_add(1, Ordering::SeqCst);
                        Reply::Query(control.query_udp_traversal(&snapshot).await?)
                    }
                    Operation::Peers { context, peers } => {
                        Reply::Peers(control.query_connectivity_peers(&context, &peers).await?)
                    }
                    Operation::Prepare { context, request } => {
                        Reply::Prepare(control.prepare_udp_traversal(&context, &request).await?)
                    }
                    Operation::Accept(offer) => {
                        Reply::Accept(control.accept_udp_traversal(&offer).await?)
                    }
                    Operation::Cancel(offer) => {
                        control.cancel_udp_traversal(&offer).await?;
                        Reply::Cancelled
                    }
                })
            }
            .await;
            if let Some(timeline) = &self.timeline {
                use meshway_control::connectivity::{AttemptView, Outcome};
                timeline.record(format_args!(
                    "rpc-end node={node} elapsed_ms={}",
                    started.elapsed().as_millis()
                ));
                match &result {
                    Ok(Reply::Peers(view)) => {
                        timeline.record(format_args!("peers node={node} view={view:?}"))
                    }
                    Ok(Reply::Query(Outcome::Current(discovery))) => {
                        for view in &discovery.attempts {
                            let (phase, offer) = match view {
                                AttemptView::Waiting(offer) => ("waiting", offer),
                                AttemptView::Active(active) => ("active", active.validated_offer()),
                            };
                            timeline.record(format_args!("query node={node} phase={phase} id={} remaining_ms={} current={:?}", offer.offer().attempt_id, offer.expires_at().saturating_duration_since(std::time::Instant::now()).as_millis(), offer.ensure_current()));
                        }
                    }
                    Ok(Reply::Prepare(Outcome::Current(offer))) => timeline.record(format_args!(
                        "prepared node={node} offer={:?} remaining_ms={}",
                        offer.offer(),
                        offer
                            .expires_at()
                            .saturating_duration_since(std::time::Instant::now())
                            .as_millis()
                    )),
                    Ok(Reply::Accept(Outcome::Current(view))) => {
                        let (phase, offer) = match view {
                            AttemptView::Waiting(offer) => ("waiting", offer),
                            AttemptView::Active(active) => ("active", active.validated_offer()),
                        };
                        timeline.record(format_args!(
                            "accepted node={node} phase={phase} id={} remaining_ms={} current={:?}",
                            offer.offer().attempt_id,
                            offer
                                .expires_at()
                                .saturating_duration_since(std::time::Instant::now())
                                .as_millis(),
                            offer.ensure_current()
                        ));
                    }
                    Ok(
                        Reply::Prepare(Outcome::Unavailable(reason))
                        | Reply::Accept(Outcome::Unavailable(reason))
                        | Reply::Query(Outcome::Unavailable(reason)),
                    ) => timeline.record(format_args!("unavailable node={node} reason={reason:?}")),
                    Err(error) => {
                        timeline.record(format_args!("rpc-error node={node} error={error:?}"))
                    }
                    _ => {}
                }
            }
            result.map_err(|_| Error::Unavailable)
        })
    }
    fn published(&self, reference: &CandidateReference) -> bool {
        self.acknowledged.lock().unwrap().contains(reference)
    }
}
impl Gateway {
    pub(super) async fn report_changes(&self, snapshot: Snapshot) {
        let candidates: Vec<_> = snapshot
            .sockets
            .iter()
            .flat_map(|socket| {
                socket
                    .endpoints
                    .iter()
                    .map(|observation| socket.candidate(observation).unwrap())
            })
            .collect();
        let changed = *self.acknowledged.lock().unwrap() != candidates;
        if changed {
            self.report(snapshot).await;
        }
    }
    pub(super) async fn synchronize(
        &self,
        manager: &mut Manager,
        secure: &mut meshway_dataplane::secure::SecureNode,
    ) -> meshway_authority::DataPlaneIdentity {
        self.install(manager, secure, true).await
    }
    pub(super) async fn install(
        &self,
        manager: &mut Manager,
        secure: &mut meshway_dataplane::secure::SecureNode,
        continuous: bool,
    ) -> meshway_authority::DataPlaneIdentity {
        let identity = {
            let mut control = self.control.lock().await;
            control.authenticate().await.unwrap();
            control.refresh_configuration().await.unwrap();
            control.acknowledge_saved().await.unwrap();
            control
                .dataplane_identity(control.execution_snapshot().unwrap())
                .unwrap()
        };
        assert!(
            !continuous || manager.can_reconcile(&identity),
            "unchanged physical resources must accept the direct Control handoff: stopped={}, lineage={}, original_bounds={}, resources={:?}, health={:?}",
            manager.shared.stop.is_cancelled(),
            identity
                .snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .is_ok(),
            manager
                .shared
                .identity
                .snapshot()
                .retention_deadline()
                .is_ok(),
            identity
                .snapshot()
                .continuation_from(manager.shared.identity.snapshot())
                .ok()
                .map(|old| {
                    let cfg = identity.snapshot().configuration().unwrap();
                    (
                        old.listeners == cfg.listeners,
                        old.dialers == cfg.dialers,
                        old.peers == cfg.peers,
                    )
                }),
            manager
                .shared
                .links
                .lock()
                .unwrap()
                .values()
                .map(|link| (
                    link.scope.health.as_ref().map(|health| health
                        .lock()
                        .unwrap()
                        .saturating_duration_since(tokio::time::Instant::now())),
                    admit_adjacency(
                        identity.snapshot().configuration().unwrap(),
                        link.fact.peer,
                        &link.fact.carrier,
                        identity.snapshot().now_ms().unwrap()
                    )
                    .is_ok()
                ))
                .collect::<Vec<_>>()
        );
        secure.reconcile(identity.clone()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), manager.reconcile(identity.clone()))
            .await
            .expect("handoff without acquisition")
            .unwrap();
        secure.set_carriers(manager.carriers().unwrap()).unwrap();
        {
            let mut control = self.control.lock().await;
            let ticket = control.begin_application().await.unwrap();
            control
                .finish_application(&ticket, Ok(()), chrono::Utc::now())
                .await
                .unwrap();
        }
        self.report(manager.snapshot().unwrap()).await;
        identity
    }

    pub(super) async fn report(&self, snapshot: Snapshot) {
        if let Some(timeline) = &self.timeline {
            timeline.record(format_args!("report sockets={:?}", snapshot.sockets));
        }
        let mut control = self.control.lock().await;
        let identity = control.execution_snapshot().unwrap();
        let cfg = identity.configuration().unwrap();
        let reference = identity.reference();
        let mut state = NodeState {
            system: SystemState {
                hostname: cfg.hostname.clone(),
                os: "test".into(),
                arch: "test".into(),
                agent_version: "test".into(),
            },
            capabilities: cfg.self_keys.capabilities.clone(),
            configuration: ConfigurationState {
                saved: Some(reference.clone()),
                applied: Some(reference),
                progress: None,
            },
            listeners: BTreeMap::new(),
            sockets: BTreeMap::new(),
            links: BTreeMap::new(),
            paths: BTreeMap::new(),
            selection: Default::default(),
            probes: Vec::new(),
            problems: Vec::new(),
        };
        let mut candidates = Vec::new();
        for socket in snapshot.sockets {
            for observation in &socket.endpoints {
                candidates.push(socket.candidate(observation).unwrap());
            }
            if let ResourceRef::Listener { key } = &socket.reference.resource {
                state.listeners.insert(
                    key.clone(),
                    ListenerState {
                        phase: ListenerPhase::Listening,
                        socket_ids: vec![socket.reference.socket_id.clone()],
                        loaded_certificate_sha256: None,
                        problem: None,
                        configuration: Some(socket.reference.configuration.clone()),
                    },
                );
            }
            state.sockets.insert(
                socket.reference.socket_id,
                SocketState {
                    resource: socket.reference.resource,
                    configuration: Some(socket.reference.configuration),
                    generation: socket.reference.socket_generation,
                    network: socket.network,
                    bound_address: socket.bound_address,
                    endpoints: socket.endpoints,
                },
            );
        }
        control
            .report_runtime_state(state, chrono::Utc::now())
            .await
            .unwrap();
        *self.acknowledged.lock().unwrap() = candidates;
    }
}
