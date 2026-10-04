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
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn rid() -> RequestId {
    RequestId::new(crate::accounts::random_uuid().unwrap()).unwrap()
}

struct HttpsSource {
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
        let mut app = App::open(Settings::new(path)).await.unwrap();
        app.resource_connector = Some(source);
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
            .json(&NodeChallengeRequest {
                network_id: network,
                node_id: node.node_id,
                key_id: node.keys.control.id,
                operation: ProofOperation::Session,
                request_digest: crate::execution::proof::request_digest(&intent).unwrap(),
            }),
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
                capabilities: super::node_tests::caps(),
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
    ) -> Result<crate::resources::service::RuleSetScope, crate::state::StateError> {
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
    ) -> Result<(), crate::state::StateError> {
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
    assert_eq!(
        s.view(token, network).await.problem,
        Some(ResourceProblem::NotReady)
    );
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
        action: RouteAction::Exit {
            nodes: vec![format!("node:{}", b.node_id)],
            via: vec![],
        },
    });
    let response = s
        .client
        .post(s.network(network, "configuration"))
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
        StatusCode::UNPROCESSABLE_ENTITY,
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
    let cross = s
        .client
        .get(s.network(outside, "rule-sets/remote"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(cross.status(), StatusCode::FORBIDDEN);
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
    assert!(pending.artifact.is_none());
    assert_eq!(pending.problem, Some(ResourceProblem::NotReady));
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
                cache: s.app.configuration_cache.clone(),
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
            .any(|entry| matches!(
                entry.change,
                crate::state::stream::CommittedChange::Resource { .. }
            ))
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
    s.close().await;
}
