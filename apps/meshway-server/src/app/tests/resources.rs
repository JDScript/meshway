//! Real HTTPS bytes, root HTTP router, signed enrollment and publication SQL.
//! Only DNS/connection pinning is injected: test TLS still verifies the hostname
//! and certificate and uses the production redirect/body/timeout checks.
use super::*;
use crate::resources::download::{Connector, ResolveFuture, client_builder};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_configuration::{execution::*, *};
use meshway_model::*;
use meshway_protocol::{configuration::*, enrollment::*, execution::*, node::*, resources::*, *};
use reqwest::{Client, RequestBuilder, StatusCode};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

fn rid() -> RequestId {
    RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap()
}

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
    // The scanner clones its provider before awaiting the serial owner. The
    // extra cache reference proves it has entered a scan behind the blocked job.
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

struct HttpsSource {
    requests: Arc<AtomicUsize>,
    hold_body: Arc<AtomicBool>,
    body_entered: Arc<tokio::sync::Notify>,
    peer_closed: Arc<AtomicBool>,
    address: SocketAddr,
    certificate: Vec<u8>,
    body: Arc<Mutex<(u16, String)>>,
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pause: AtomicBool,
    entered: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    private_dns: AtomicBool,
}
impl HttpsSource {
    fn new() -> Arc<Self> {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["rules.meshway.example".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let certificate = cert.der().to_vec();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let body = Arc::new(Mutex::new((
            200u16,
            String::from("payload:\n - 8.8.8.0/25\n"),
        )));
        let stopped = Arc::new(AtomicBool::new(false));
        let server_body = body.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let hold_body = Arc::new(AtomicBool::new(false));
        let held = hold_body.clone();
        let body_entered = Arc::new(tokio::sync::Notify::new());
        let entered = body_entered.clone();
        let peer_closed = Arc::new(AtomicBool::new(false));
        let closed = peer_closed.clone();
        let stop = stopped.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let (socket, _) = match listener.accept() {
                    Ok(socket) => socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut tls = rustls::StreamOwned::new(
                    rustls::ServerConnection::new(config.clone()).unwrap(),
                    socket,
                );
                let mut input = Vec::new();
                let mut buf = [0; 1024];
                while !input.windows(4).any(|v| v == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => input.extend_from_slice(&buf[..n]),
                    }
                }
                if input.is_empty() {
                    continue;
                }
                server_requests.fetch_add(1, Ordering::SeqCst);
                if held.load(Ordering::SeqCst) {
                    entered.notify_one();
                    tls.sock
                        .set_read_timeout(Some(Duration::from_millis(100)))
                        .unwrap();
                    while held.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                        match tls.read(&mut buf) {
                            Ok(0) => {
                                closed.store(true, Ordering::SeqCst);
                                break;
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) => {}
                            Err(_) => {
                                closed.store(true, Ordering::SeqCst);
                                break;
                            }
                            Ok(_) => {}
                        }
                    }
                    if closed.load(Ordering::SeqCst) {
                        continue;
                    }
                }
                let (status, body) = server_body.lock().unwrap().clone();
                let header = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nLocation: https://127.0.0.1/forbidden\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = tls
                    .write_all(header.as_bytes())
                    .and_then(|_| tls.write_all(body.as_bytes()))
                    .and_then(|_| tls.flush());
            }
        });
        Arc::new(Self {
            requests,
            hold_body,
            body_entered,
            peer_closed,
            address,
            certificate,
            body,
            stopped,
            thread: Some(thread),
            pause: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
            private_dns: AtomicBool::new(false),
        })
    }
    fn set(&self, status: u16, body: &str) {
        *self.body.lock().unwrap() = (status, body.into());
    }
    fn pause(&self) {
        self.pause.store(true, Ordering::SeqCst);
    }
    fn url(&self, path: &str) -> String {
        format!(
            "https://rules.meshway.example:{}{path}",
            self.address.port()
        )
    }
    fn definition(&self) -> RuleSet {
        serde_json::from_str(&format!(
            r#"{{"type":"remote","url":"{}","format":"mihomo-ipcidr"}}"#,
            self.url("/rules")
        ))
        .unwrap()
    }
}
impl Drop for HttpsSource {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
impl Connector for HttpsSource {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            assert_eq!(host, "rules.meshway.example");
            if self.pause.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.resume.notified().await;
            }
            let ip = if self.private_dns.load(Ordering::SeqCst) {
                "127.0.0.1"
            } else {
                "93.184.216.34"
            };
            Ok(vec![SocketAddr::new(ip.parse().unwrap(), port)])
        })
    }
    fn client(&self, host: &str, addresses: &[SocketAddr]) -> Result<Client, ResourceProblem> {
        assert_eq!(
            addresses,
            [SocketAddr::new(
                "93.184.216.34".parse().unwrap(),
                self.address.port()
            )]
        );
        client_builder(host, &[self.address])
            .add_root_certificate(reqwest::Certificate::from_der(&self.certificate).unwrap())
            .build()
            .map_err(|_| ResourceProblem::DownloadFailed)
    }
}

struct Server {
    app: App,
    url: String,
    client: Client,
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn open(path: &std::path::Path, source: Arc<HttpsSource>) -> Self {
        let app = App::open_with_downloads(
            Settings::new(path),
            crate::resources::downloads::Downloads::with_connector(source),
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let router = crate::http::router(&app);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        Self {
            app,
            url,
            client: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            stop,
            task,
        }
    }
    fn network(&self, id: NetworkId, path: &str) -> String {
        format!("{}/api/v1/networks/{id}/{path}", self.url)
    }
    async fn register(&self, email: &str) -> SessionIssuedResponse {
        json(
            self.client
                .post(format!("{}/api/v1/auth/register", self.url))
                .json(&RegisterRequest {
                    email: email.into(),
                    password: WireSecret::new("a sufficiently long password".into()),
                }),
        )
        .await
    }
    async fn create(&self, token: &str) -> NetworkSummary {
        json::<NetworkResponse>(
            self.client
                .post(format!("{}/api/v1/networks", self.url))
                .bearer_auth(token)
                .json(&CreateNetworkRequest {
                    name: "resources".into(),
                    request_id: rid(),
                }),
        )
        .await
        .network
    }
    async fn source(&self, token: &str, network: NetworkId) -> ConfigurationSourceResponse {
        json(
            self.client
                .get(self.network(network, "configuration/source"))
                .bearer_auth(token),
        )
        .await
    }
    async fn apply(
        &self,
        token: &str,
        network: NetworkId,
        revision: Revision,
        input: &NetworkConfigurationInput,
    ) -> ConfigurationAppliedResponse {
        json(
            self.client
                .post(self.network(network, "configuration"))
                .bearer_auth(token)
                .json(&ConfigurationRequest::Document {
                    expected_revision: revision,
                    request_id: rid(),
                    source_jsonc: WireSecret::new(serde_json::to_string(input).unwrap()),
                }),
        )
        .await
    }
    async fn refresh(&self, token: &str, network: NetworkId) -> RuleSetView {
        json(
            self.client
                .post(self.network(network, "rule-sets/remote/refresh"))
                .bearer_auth(token),
        )
        .await
    }
    async fn view(&self, token: &str, network: NetworkId) -> RuleSetView {
        json::<RuleSetList>(
            self.client
                .get(self.network(network, "rule-sets/remote"))
                .bearer_auth(token),
        )
        .await
        .items
        .remove(0)
    }
    async fn close(self) {
        self.app.stream_shutdown()();
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
        self.app.shutdown().await.unwrap();
    }
}
async fn json<T: serde::de::DeserializeOwned>(request: RequestBuilder) -> T {
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("{error}: {body}"))
}

async fn enroll(
    s: &Server,
    token: &str,
    network: NetworkId,
    seed: u8,
) -> (EnrollmentResponse, String, InstanceRequest) {
    let signer = SigningKey::from_bytes(&[seed; 32]);
    let issued: EnrollmentKeyResponse = json(
        s.client
            .post(s.network(network, "enrollment-keys"))
            .bearer_auth(token)
            .json(&CreateEnrollmentKeyRequest {
                request_id: rid(),
                tags: vec!["tag:test".into()],
                expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 600_000,
                initial_settings: InitialNodeSettings {
                    hostname: Some(format!("node-{seed}")),
                    listeners: serde_json::from_str(
                        r#"{"udp":{"type":"native_udp","listenPort":23456}}"#,
                    )
                    .unwrap(),
                },
            }),
    )
    .await;
    let verifier = "x".repeat(43);
    let intent = EnrollmentIntent {
        id: EnrollmentRequestId::new(*rid().as_uuid()).unwrap(),
        existing: None,
        control_key: NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: signer.verifying_key().to_bytes().into(),
        },
        dataplane_key: NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: [seed + 20; 32].into(),
        },
        code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
        hostname: format!("node-{seed}"),
    };
    let signature = signer.sign(&enrollment_transcript(
        intent.id,
        crate::execution::proof::request_digest(&intent).unwrap(),
        "create",
    ));
    let node: EnrollmentResponse = json(
        s.client
            .post(format!("{}/agent-api/v1/enrollments", s.url))
            .json(&KeyEnrollmentRequest {
                request: CreateEnrollmentRequest {
                    intent,
                    signature: WireSecret::new(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
                },
                verifier: WireSecret::new(verifier),
                enrollment_key: issued.secret.unwrap(),
            }),
    )
    .await;
    let intent = NodeSessionIntent { request_id: rid() };
    let challenge: NodeChallenge = json(
        s.client
            .post(format!("{}/agent-api/v1/auth/challenges", s.url))
            .json(&s.app.execution().signed_challenge_fixture(
                NodeChallengeRequest {
                    network_id: network,
                    node_id: node.node_id,
                    key_id: node.keys.control.id,
                    operation: ProofOperation::Session,
                    request_digest: crate::execution::proof::request_digest(&intent).unwrap(),
                },
                &signer,
            )),
    )
    .await;
    let session: NodeSessionResponse = json(
        s.client
            .post(format!("{}/agent-api/v1/auth/session", s.url))
            .json(&NodeSessionRequest {
                intent,
                proof: NodeProof {
                    challenge_id: challenge.id,
                    signature: WireSecret::new(
                        URL_SAFE_NO_PAD
                            .encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
                    ),
                },
            }),
    )
    .await;
    let instance: InstanceLease = json(
        s.client
            .post(format!("{}/agent-api/v1/instances", s.url))
            .bearer_auth(session.token.expose())
            .json(&AcquireInstanceRequest {
                instance_id: InstanceId::new(*rid().as_uuid()).unwrap(),
                capabilities: super::node::caps(),
            }),
    )
    .await;
    (
        node,
        session.token.expose().into(),
        InstanceRequest {
            instance_id: instance.instance_id,
            generation: instance.generation,
        },
    )
}

async fn target(s: &Server, token: &str, instance: &InstanceRequest) -> NodeConfigurationResponse {
    json(
        s.client
            .post(format!("{}/agent-api/v1/configuration/query", s.url))
            .bearer_auth(token)
            .json(&NodeConfigurationRequest {
                instance: instance.clone(),
                base: None,
            }),
    )
    .await
}
async fn publication_bindings(
    s: &Server,
    network: NetworkId,
    publication: PublicationId,
) -> BTreeMap<String, ArtifactId> {
    s.app.executor.handle().execute(move |db| {
        let bytes: Vec<u8> = db.query_row("SELECT rule_set_bindings FROM execution_publications WHERE network_id=?1 AND id=?2", rusqlite::params![network.as_uuid().as_bytes().as_slice(), publication.as_uuid().as_bytes().as_slice()], |row| row.get(0)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }).await.unwrap()
}

async fn resource_counts(s: &Server, network: NetworkId) -> (i64, i64, i64) {
    s.app
        .executor
        .handle()
        .execute(move |db| {
            let count = |table: &str| {
                db.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE network_id=?1"),
                    [network.as_uuid().as_bytes().as_slice()],
                    |row| row.get(0),
                )
                .unwrap()
            };
            (
                count("dataset_artifacts"),
                count("configuration_versions"),
                count("execution_publications"),
            )
        })
        .await
        .unwrap()
}
async fn wait_view(
    s: &Server,
    token: &str,
    network: NetworkId,
    predicate: impl Fn(&RuleSetView) -> bool,
) -> RuleSetView {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let view = s.view(token, network).await;
            if predicate(&view) {
                return view;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("resource scheduler made bounded progress")
}

#[tokio::test]
async fn configuration_prepares_new_and_replaced_references_in_one_cas_without_preview_writes() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("apply-resources@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (a, at, ai) = enroll(&s, token, network, 41).await;
    let (b, _, _) = enroll(&s, token, network, 42).await;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.rule_sets.insert("remote".into(), remote.definition());
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["8.8.8.0/24".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let before = resource_counts(&s, network).await;
    let preview: ConfigurationPreviewResponse = json(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    assert_eq!(preview.prepared_rule_sets["remote"].entries, 1);
    assert_eq!(resource_counts(&s, network).await, before);
    assert_eq!(s.source(token, network).await.revision, current.revision);
    let applied = s.apply(token, network, current.revision, &input).await;
    assert_eq!(applied.revision.get(), current.revision.get() + 1);
    let first = s.view(token, network).await.artifact.unwrap().id;
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first
    );
    let demand_id = rid();
    let result: DemandResponse = json(
        s.client
            .post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: ai.clone(),
                request_id: demand_id,
                target: BusinessTarget::Ip {
                    address: ExecutionAddress("8.8.8.200".parse().unwrap()),
                },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 300,
            }),
    )
    .await;
    assert!(matches!(
        result.decision,
        DemandDecision::Unmanaged | DemandDecision::Direct
    ));
    let current = s.source(token, network).await;
    let mut replacement = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = replacement.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/replacement");
    }
    remote.set(200, "payload:\n - 0.0.0.0/0\n");
    let before = resource_counts(&s, network).await;
    for endpoint in ["configuration/validate", "configuration"] {
        let response = s
            .client
            .post(s.network(network, endpoint))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&replacement).unwrap()),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(resource_counts(&s, network).await, before);
        assert_eq!(
            s.source(token, network).await.source_jsonc.expose(),
            current.source_jsonc.expose()
        );
    }
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let replaced = s
        .apply(token, network, current.revision, &replacement)
        .await;
    let second = s.view(token, network).await.artifact.unwrap().id;
    assert_ne!(first, second);
    let target = target(&s, &at, &ai).await.full.unwrap().configuration;
    assert!(matches!(
        target.decisions[&demand_id],
        DemandDecision::Permitted { .. }
    ));
    assert_eq!(
        publication_bindings(&s, network, target.publication).await["remote"],
        second
    );
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first
    );

    // A normal concurrent source edit wins its CAS; the downloaded candidate
    // cannot overwrite that newer complete configuration.
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/racing");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: replaced.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    let mut other = NetworkConfigurationInput::from(&current.configuration);
    other.nodes.get_mut(&a.node_id).unwrap().remark = "concurrent edit".into();
    s.apply(token, network, current.revision, &other).await;
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, second);

    // Same source revision, newer bound content: the final transaction also
    // rejects the stale base artifact rather than relying solely on source CAS.
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/artifact-race");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    remote.set(200, "payload:\n - 8.8.8.128/25\n");
    let refreshed = s.refresh(token, network).await.artifact.unwrap().id;
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::CONFLICT
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, refreshed);
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        current.source_jsonc.expose()
    );
    // Both apply and validate recheck the actual manager role after shared I/O.
    use meshway_protocol::management::*;
    let admin = s.register("apply-resource-admin@example.com").await;
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
        panic!("join secret");
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
    let current = s.source(token, network).await;
    let mut candidate = NetworkConfigurationInput::from(&current.configuration);
    if let RuleSet::Remote { url, .. } = candidate.rule_sets.get_mut("remote").unwrap() {
        *url = remote.url("/role-race");
    }
    remote.pause();
    let pending = tokio::spawn(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(admin.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    remote.entered.notified().await;
    let preview = tokio::spawn(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(admin.token.expose())
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&candidate).unwrap()),
            })
            .send(),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
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
    assert!(response.status().is_success());
    let counts = resource_counts(&s, network).await;
    remote.resume.notify_one();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        preview.await.unwrap().unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(resource_counts(&s, network).await, counts);
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        current.source_jsonc.expose()
    );
    // Per-file valid inputs cannot multiply into an unbounded execution input
    // through aliases or successive configuration versions.
    let mut body = String::from("payload:\n");
    let half = u32::try_from(crate::resources::MAX_RULE_ENTRIES / 2).unwrap();
    for index in 0..=half {
        body.push_str(&format!(
            " - {}/32\n",
            std::net::Ipv4Addr::from(0x0b000000 + index)
        ));
    }
    remote.set(200, &body);
    let mut large = NetworkConfigurationInput::from(&current.configuration);
    let mut definition = remote.definition();
    if let RuleSet::Remote { url, .. } = &mut definition {
        *url = remote.url("/large");
    }
    large.rule_sets.insert("large-a".into(), definition.clone());
    large.rule_sets.insert("large-b".into(), definition);
    let before = resource_counts(&s, network).await;
    for endpoint in ["configuration/validate", "configuration"] {
        let response = s
            .client
            .post(s.network(network, endpoint))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&large).unwrap()),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let error: ErrorResponse = response.json().await.unwrap();
        let Problem::ConfigurationInvalid { params, .. } = error.problem else {
            panic!("typed budget error");
        };
        assert!(
            params
                .issues
                .iter()
                .any(|issue| issue.code == IssueCode::RuleSetBudget
                    && issue.path == "/ruleSets"
                    && issue.params.contains_key("maxEntries"))
        );
        assert_eq!(resource_counts(&s, network).await, before);
    }
    // Too many remote sets in one edit is the same kind of diagnostic, not a
    // server outage.
    let mut many = NetworkConfigurationInput::from(&current.configuration);
    for index in 0..=crate::resources::MAX_REMOTE_RULE_SETS {
        let mut definition = remote.definition();
        if let RuleSet::Remote { url, .. } = &mut definition {
            *url = remote.url(&format!("/many/{index}"));
        }
        many.rule_sets.insert(format!("many-{index}"), definition);
    }
    let response = s
        .client
        .post(s.network(network, "configuration/validate"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: current.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&many).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let error: ErrorResponse = response.json().await.unwrap();
    let Problem::ConfigurationInvalid { params, .. } = error.problem else {
        panic!("typed budget error");
    };
    assert_eq!(params.issues.len(), 1);
    assert_eq!(params.issues[0].code, IssueCode::RuleSetBudget);
    assert_eq!(
        params.issues[0].params["maxSets"],
        crate::resources::MAX_REMOTE_RULE_SETS.to_string()
    );
    assert_eq!(resource_counts(&s, network).await, before);
    s.close().await;
}

#[tokio::test]
async fn exact_names_in_remote_lists_bind_under_an_internet_grant() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    // A MetaCubeX-style geosite list: suffixes and bare exact names mixed.
    remote.set(
        200,
        "payload:\n - '+.telegram.org'\n - 'telegram.me'\n - 't.me'\n - '+.t.me'\n",
    );
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("exact-names@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let (_, _, _) = enroll(&s, token, network, 51).await;
    let (b, _, _) = enroll(&s, token, network, 52).await;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.rule_sets.insert(
        "remote".into(),
        serde_json::from_str(&format!(
            r#"{{"type":"remote","url":"{}","format":"mihomo-domain"}}"#,
            remote.url("/geosite")
        ))
        .unwrap(),
    );
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["autogroup:internet".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let preview: ConfigurationPreviewResponse = json(
        s.client
            .post(s.network(network, "configuration/validate"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Document {
                expected_revision: current.revision,
                request_id: rid(),
                source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
            }),
    )
    .await;
    assert_eq!(preview.prepared_rule_sets["remote"].entries, 4);
    s.apply(token, network, current.revision, &input).await;
    let view = s.view(token, network).await;
    assert_eq!(view.problem, None);
    let artifact = view.artifact.unwrap();
    assert_eq!(artifact.entries, 4);
    let data = s
        .app
        .executor
        .handle()
        .read(move |tx| crate::resources::load_artifact(tx, network, artifact.id))
        .await
        .unwrap();
    assert_eq!(data.domain_suffix, vec!["t.me", "telegram.org"]);
    assert_eq!(data.domain, vec!["t.me", "telegram.me"]);
    assert!(data.ip_cidr.is_empty());
    // The one mihomo form without an equivalent still fails the whole list.
    remote.set(200, "payload:\n - '*.telegram.org'\n");
    let failed = s.refresh(token, network).await;
    assert_eq!(failed.problem, Some(ResourceProblem::ConversionFailed));
    assert_eq!(failed.artifact.unwrap().id, artifact.id);
    s.close().await;
}

#[tokio::test]
async fn startup_scheduler_updates_backs_off_persists_and_cancels_its_tls_socket() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let owner = s.register("scheduler@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
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
    let first = initial.artifact.unwrap().id;
    s.close().await;
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    // No explicit refresh or retained administrator session is passed to the
    // worker: ordinary App startup resumes from persisted check deadlines.
    let s = Server::open(&path, remote.clone()).await;
    let updated = wait_view(&s, token, network, |v| {
        v.artifact.as_ref().is_some_and(|a| a.id != first)
    })
    .await;
    let counts = resource_counts(&s, network).await;
    remote.set(200, "payload:\n - 8.8.8.9/24\n - 8.8.8.0/24\n");
    let unchanged = wait_view(&s, token, network, |v| v.checked_at > updated.checked_at).await;
    assert_eq!(unchanged.publication, updated.publication);
    assert_eq!(resource_counts(&s, network).await, counts);
    remote.set(503, "unavailable");
    let failed = wait_view(&s, token, network, |v| {
        v.problem == Some(ResourceProblem::DownloadFailed)
    })
    .await;
    let requests = remote.requests.load(Ordering::SeqCst);
    s.close().await;
    let s = Server::open(&path, remote.clone()).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(s.view(token, network).await.checked_at, failed.checked_at);
    assert_eq!(remote.requests.load(Ordering::SeqCst), requests);
    let retried = wait_view(&s, token, network, |v| v.checked_at > failed.checked_at).await;
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
async fn execution_facts(
    s: &Server,
    network: NetworkId,
    node: NodeId,
) -> (Vec<u8>, Option<i64>, i64) {
    s.app.executor.handle().execute(move |db| db.query_row(
        "SELECT target_publication_id,execution_expires_at,(SELECT count(*) FROM execution_receipts r WHERE r.network_id=e.network_id AND r.node_id=e.node_id AND r.publication_id=e.target_publication_id) FROM node_execution e WHERE network_id=?1 AND node_id=?2",
        rusqlite::params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()).await.unwrap()
}

struct CommitGate {
    provider: crate::resources::provider::Provider,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl crate::resources::service::ResourceProvider for CommitGate {
    fn rule_sets(
        &self,
        tx: &rusqlite::Transaction<'_>,
        identity: &crate::state::service::ReadIdentity,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<crate::resources::service::RuleSetScope, crate::error::Error> {
        self.provider.rule_sets(tx, identity, now)
    }
    fn bind_rule_set(
        &self,
        tx: &rusqlite::Transaction<'_>,
        network: NetworkId,
        revision: Revision,
        key: &str,
        artifact: &ArtifactSummary,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), crate::error::Error> {
        self.provider
            .bind_rule_set(tx, network, revision, key, artifact, now)?;
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(())
    }
}

#[tokio::test]
async fn remote_https_artifacts_compile_publish_restore_and_survive_restart() {
    let _serial = crate::accounts::TEST_KDF_LOCK.lock().await;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("data");
    let remote = HttpsSource::new();
    let s = Server::open(&path, remote.clone()).await;
    let probe = remote
        .client(
            "rules.meshway.example",
            &[SocketAddr::new(
                "93.184.216.34".parse().unwrap(),
                remote.address.port(),
            )],
        )
        .unwrap()
        .get(remote.url("/rules"))
        .send()
        .await
        .expect("fixture TLS must verify");
    assert_eq!(probe.text().await.unwrap(), "payload:\n - 8.8.8.0/25\n");
    let owner = s.register("resource-owner@example.com").await;
    let token = owner.token.expose();
    let network = s.create(token).await.id;
    let source = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    input.tags_owners.insert("tag:test".into(), vec![]);
    input.rule_sets.insert("remote".into(), remote.definition());
    s.apply(token, network, source.revision, &input).await;
    assert!(s.view(token, network).await.problem.is_none());
    let (a, at, ai) = enroll(&s, token, network, 31).await;
    let (b, bt, bi) = enroll(&s, token, network, 32).await;
    let (c, ct, ci) = enroll(&s, token, network, 33).await;
    let source = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    input.nodes.get_mut(&b.node_id).unwrap().exit_enabled = true;
    input.topology.push(TopologyRule {
        remark: String::new(),
        action: TopologyAction::Block,
        a: format!("node:{}", c.node_id),
        b: "*".into(),
    });
    input.access.push(AccessRule {
        remark: String::new(),
        src: vec!["*".into()],
        dst: vec!["8.8.8.0/24".into()],
        ip: vec!["*".into()],
        bidirectional: false,
    });
    input.routing.egress.rules.push(EgressRule {
        remark: String::new(),
        matcher: RouteMatch {
            rule_sets: vec!["remote".into()],
            ..Default::default()
        },
        action: RuleAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let response = s
        .client
        .post(s.network(network, "configuration/validate"))
        .bearer_auth(token)
        .json(&ConfigurationRequest::Document {
            expected_revision: source.revision,
            request_id: rid(),
            source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response.text().await.unwrap()
    );
    let first = s.refresh(token, network).await;
    assert!(first.problem.is_none(), "{:?}", first.problem);
    let first_id = first.artifact.unwrap().id;
    let applied = s.apply(token, network, source.revision, &input).await;
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first_id
    );
    let fixed_revision = applied.revision;
    let source_before = s.source(token, network).await;
    let demand_id = rid();
    let demand: DemandResponse = json(
        s.client
            .post(format!("{}/agent-api/v1/demands", s.url))
            .bearer_auth(&at)
            .json(&DemandRequest {
                instance: ai.clone(),
                request_id: demand_id,
                target: BusinessTarget::Ip {
                    address: ExecutionAddress("8.8.8.200".parse().unwrap()),
                },
                protocol: BusinessProtocol::Tcp,
                port: Some(443.try_into().unwrap()),
                profile: "mesh".into(),
                duration_seconds: 300,
            }),
    )
    .await;
    assert!(matches!(
        demand.decision,
        DemandDecision::Unmanaged | DemandDecision::Direct
    ));
    let _: InstanceLease = json(
        s.client
            .post(format!("{}/agent-api/v1/instances/renew", s.url))
            .bearer_auth(&ct)
            .json(&ci),
    )
    .await;
    let unrelated = target(&s, &ct, &ci).await;
    let response = s
        .client
        .post(format!("{}/agent-api/v1/configuration/receipts", s.url))
        .bearer_auth(&ct)
        .json(&ReceiptRequest {
            instance: ci.clone(),
            target: unrelated.lease.target.unwrap(),
            phase: ReceiptPhase::Saved,
            attempt: Version::MIN,
            sequence: Sequence::MIN,
            reported_at_ms: chrono::Utc::now().timestamp_millis() as u64,
            failure: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let unrelated_facts = execution_facts(&s, network, c.node_id).await;
    assert!(unrelated_facts.1.is_some());
    assert_eq!(unrelated_facts.2, 1);
    // Subscribe through the real stream, finish its initial batch, then refresh.
    let mut events = s
        .client
        .get(s.network(network, "events"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let mut stream_text = String::new();
    while !stream_text.contains("sync.ready") {
        stream_text.push_str(std::str::from_utf8(&events.chunk().await.unwrap().unwrap()).unwrap());
    }
    remote.set(200, "payload:\n - 8.8.8.0/24\n");
    let second = s.refresh(token, network).await;
    assert!(second.problem.is_none());
    let last_success = second.last_success_at;
    assert!(last_success.is_some());
    let second_publication = second.publication.unwrap();
    let second_id = second.artifact.unwrap().id;
    assert_ne!(first_id, second_id);
    assert_eq!(
        execution_facts(&s, network, c.node_id).await,
        unrelated_facts
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        while !stream_text.contains("resources.changed") {
            stream_text
                .push_str(std::str::from_utf8(&events.chunk().await.unwrap().unwrap()).unwrap());
        }
    })
    .await
    .unwrap();
    assert!(
        !stream_text
            .lines()
            .find(|line| line.contains("resources.changed"))
            .unwrap()
            .contains("rules.meshway.example")
    );
    drop(events);
    let after = target(&s, &at, &ai).await;
    let configuration = after.full.unwrap().configuration;
    assert!(
        !serde_json::to_string(&configuration)
            .unwrap()
            .contains("rules.meshway.example")
    );
    assert!(matches!(
        configuration.decisions[&demand_id],
        DemandDecision::Permitted { .. }
    ));
    assert!(configuration.permits.values().any(|permit| {
        permit
            .paths
            .iter()
            .any(|path| path.nodes == vec![a.node_id, b.node_id])
    }));
    assert_eq!(
        publication_bindings(&s, network, configuration.publication).await["remote"],
        second_id
    );
    assert_eq!(
        s.source(token, network).await.source_jsonc.expose(),
        source_before.source_jsonc.expose()
    );
    assert_eq!(s.source(token, network).await.revision, fixed_revision);
    // Failed content conversion and widening do not overwrite the last accepted
    // target, source version, immutable content, or historical binding.
    for (status, body, problem) in [
        (503, "unavailable", ResourceProblem::DownloadFailed),
        (
            200,
            "payload:\n - not-a-cidr\n",
            ResourceProblem::ConversionFailed,
        ),
        (
            200,
            "payload:\n - 0.0.0.0/0\n",
            ResourceProblem::ValidationFailed,
        ),
        (302, "redirect", ResourceProblem::DownloadFailed),
    ] {
        remote.set(status, body);
        let failed = s.refresh(token, network).await;
        assert_eq!(failed.last_success_at, last_success);
        assert!(failed.checked_at >= last_success);
        assert_eq!(failed.problem, Some(problem));
        assert_eq!(failed.artifact.unwrap().id, second_id);
        assert_eq!(s.view(token, network).await.problem, Some(problem));
    }
    assert_eq!(
        publication_bindings(&s, network, applied.publication_ids[0]).await["remote"],
        first_id
    );
    let before_unrelated = target(&s, &bt, &bi).await;
    assert!(before_unrelated.lease.target.is_some());
    // Default restore uses initial bindings, never an implicit latest download.
    // A caller can explicitly select an exact historical execution publication.
    let source = s.source(token, network).await;
    let mut changed = NetworkConfigurationInput::from(&source.configuration);
    changed.nodes.get_mut(&a.node_id).unwrap().remark = "new config".into();
    let current = s.apply(token, network, source.revision, &changed).await;
    assert_eq!(s.view(token, network).await.last_success_at, last_success);
    remote.set(200, "payload:\n - 8.8.8.0/25\n");
    assert_eq!(
        s.refresh(token, network).await.artifact.unwrap().id,
        first_id
    );
    let initial_restore: ConfigurationAppliedResponse = json(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Restore {
                expected_revision: current.revision,
                request_id: rid(),
                restore_from: fixed_revision,
                resource_publication: None,
            }),
    )
    .await;
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, first_id);
    let restored: ConfigurationAppliedResponse = json(
        s.client
            .post(s.network(network, "configuration"))
            .bearer_auth(token)
            .json(&ConfigurationRequest::Restore {
                expected_revision: initial_restore.revision,
                request_id: rid(),
                restore_from: fixed_revision,
                resource_publication: Some(second_publication),
            }),
    )
    .await;
    assert_eq!(
        publication_bindings(&s, network, restored.publication_ids[0]).await["remote"],
        second_id
    );
    // A restored exact binding is not immediately replaced just because its
    // new source version has no download check record. Its declared interval
    // starts at this source commit without claiming a fresh download in State.
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert!(s.view(token, network).await.checked_at.is_none());
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, second_id);
    let source = s.source(token, network).await;
    let mut disabled = NetworkConfigurationInput::from(&source.configuration);
    disabled.nodes.get_mut(&b.node_id).unwrap().enabled = false;
    s.apply(token, network, source.revision, &disabled).await;
    let blocked = target(&s, &at, &ai).await.full.unwrap().configuration;
    assert!(matches!(
        blocked.decisions[&demand_id],
        DemandDecision::Unavailable { .. }
    ));
    assert!(
        blocked.permits.is_empty(),
        "missing mandatory exit cannot become direct fallback"
    );
    s.app.check_storage().await.unwrap();
    s.close().await;
    let restarted = Server::open(&path, remote.clone()).await;
    assert_eq!(
        restarted.view(token, network).await.artifact.unwrap().id,
        second_id
    );
    assert_eq!(
        publication_bindings(&restarted, network, restored.publication_ids[0]).await["remote"],
        second_id
    );
    restarted.app.check_storage().await.unwrap();
    restarted.close().await;
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
    let mut input = NetworkConfigurationInput::from(&source.configuration);
    input.rule_sets.insert("remote".into(), remote.definition());
    s.apply(token, network, source.revision, &input).await;
    let first = s.refresh(token, network).await.artifact.unwrap().id;
    // Concurrent callers join one real TLS transfer; only the first final
    // checker can update freshness, the stale waiter cannot overwrite it.
    remote.pause();
    let count = remote.requests.load(Ordering::SeqCst);
    let one = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(token)
            .send(),
    );
    remote.entered.notified().await;
    let two = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(token)
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
            .send(),
    );
    remote.entered.notified().await;
    // This normal configuration request completes while DNS/download is paused.
    let source = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&source.configuration);
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
    assert_eq!(pending.artifact.unwrap().id, first);
    assert!(pending.problem.is_none());
    assert_eq!(s.refresh(token, network).await.artifact.unwrap().id, first);

    remote.pause();
    let refresh = tokio::spawn(
        s.client
            .post(s.network(network, "rule-sets/remote/refresh"))
            .bearer_auth(admin.token.expose())
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
    remote.set(200, &"x".repeat(crate::resources::MAX_DATASET_BYTES + 1));
    assert_eq!(
        s.refresh(token, network).await.problem,
        Some(ResourceProblem::TooLarge)
    );
    assert_eq!(s.view(token, network).await.artifact.unwrap().id, first);
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
    let accepted =
        tokio::spawn(async move { service.refresh_rule_set(identity, "remote".into()).await });
    started.await.unwrap();
    accepted.abort();
    assert!(accepted.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    let committed = s.view(token, network).await;
    assert!(committed.problem.is_none());
    let committed_id = committed.artifact.unwrap().id;
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
        s.view(token, network).await.artifact.unwrap().id,
        committed_id
    );
    // An unrelated edit keeps the failed check with the unchanged binding:
    // the new version does not pretend the set was never checked.
    let failed = s.view(token, network).await;
    let current = s.source(token, network).await;
    let mut input = NetworkConfigurationInput::from(&current.configuration);
    input.tags_owners.insert("tag:later".into(), vec![]);
    s.apply(token, network, current.revision, &input).await;
    let carried = s.view(token, network).await;
    assert_eq!(carried.problem, Some(ResourceProblem::DownloadFailed));
    assert_eq!(carried.checked_at, failed.checked_at);
    assert_eq!(carried.last_success_at, failed.last_success_at);
    assert_eq!(carried.artifact.unwrap().id, committed_id);
    s.close().await;
}
