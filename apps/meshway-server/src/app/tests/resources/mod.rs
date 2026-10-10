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

struct HttpsSource {
    requests: Arc<AtomicUsize>,
    request_times: Arc<Mutex<Vec<chrono::DateTime<chrono::Utc>>>>,
    hold_body: Arc<AtomicBool>,
    body_entered: Arc<tokio::sync::Notify>,
    peer_closed: Arc<AtomicBool>,
    address: SocketAddr,
    certificate: String,
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
        let certificate = cert.pem();
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
        let request_times = Arc::new(Mutex::new(Vec::new()));
        let server_request_times = request_times.clone();
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
                server_request_times
                    .lock()
                    .unwrap()
                    .push(chrono::Utc::now());
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
            request_times,
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
        let tls = meshway_tls::Client::new(&meshway_transport_spec::ClientTls {
            server_name: host.into(),
            certificate_chain: meshway_transport_spec::SecretString::new(self.certificate.clone()),
            options: meshway_transport_spec::DialerTls {
                alpn: vec!["http/1.1".into()],
                ..Default::default()
            },
        })
        .unwrap();
        client_builder(host, &[self.address])?
            .tls_backend_preconfigured(tls.configuration())
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
                .tls_backend_preconfigured(meshway_tls::platform_client_config().unwrap())
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
                .bearer_auth(token)
                .json(&RefreshRuleSetRequest { request_id: rid() }),
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
    ) -> Result<Vec<crate::effect::Effect>, crate::error::Error> {
        let effects = self
            .provider
            .bind_rule_set(tx, network, revision, key, artifact, now)?;
        self.entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(effects)
    }
}

mod configuration;

mod lifecycle;

mod netmap_bindings;

mod publication;
