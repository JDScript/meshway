use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::*;
use meshway_protocol::{enrollment::*, execution::*, node::*, *};
use meshway_server::{App, Settings, execution::proof};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

pub fn id<T: TryFrom<uuid::Uuid>>() -> T
where
    T::Error: std::fmt::Debug,
{
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    T::try_from(uuid::Uuid::from_bytes(bytes)).unwrap()
}
fn signed_challenge(
    controller: KeyId,
    request: NodeChallengeRequest,
    signer: &SigningKey,
) -> SignedNodeChallengeRequest {
    let intent = NodeChallengeIntent {
        id: id(),
        controller_id: controller,
        request,
        expires_at_ms: chrono::Utc::now().timestamp_millis() as u64 + 30_000,
    };
    let signature = signer.sign(&challenge_request_transcript(&intent));
    SignedNodeChallengeRequest {
        intent,
        signature: WireSecret::new(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
    }
}
pub struct Server {
    pub app: App,
    pub base: String,
    pub client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
    _temp: tempfile::TempDir,
}
impl Server {
    pub async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        Self::open(temp).await
    }
    async fn open(temp: tempfile::TempDir) -> Self {
        let app = App::open(Settings::new(
            temp.path().canonicalize().unwrap().join("data"),
        ))
        .await
        .unwrap();
        let router = meshway_server::http::router(&app);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            app,
            base,
            client: reqwest::Client::builder()
                .tls_backend_preconfigured(
                    meshway_transport_stream::tls::platform_client_config().unwrap(),
                )
                .build()
                .unwrap(),
            task,
            _temp: temp,
        }
    }
    pub fn post<T: Serialize>(&self, path: &str, token: &str, body: &T) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}{path}", self.base))
            .bearer_auth(token)
            .json(body)
    }
    pub fn get(&self, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{path}", self.base))
            .bearer_auth(token)
    }
    pub async fn user(&self, email: &str) -> SessionIssuedResponse {
        json(
            self.post(
                "/api/v1/auth/register",
                "",
                &RegisterRequest {
                    email: email.into(),
                    password: WireSecret::new("long observer test password".into()),
                },
            ),
            201,
        )
        .await
    }
    pub async fn network(&self, user: &SessionIssuedResponse) -> NetworkSummary {
        let result: NetworkResponse = json(
            self.post(
                "/api/v1/networks",
                user.token.expose(),
                &CreateNetworkRequest {
                    request_id: id(),
                    name: "observation network".into(),
                },
            ),
            201,
        )
        .await;
        result.network
    }
    pub async fn enroll(
        &self,
        user: &SessionIssuedResponse,
        network: NetworkId,
        name: &str,
    ) -> Node {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).unwrap();
        let signer = SigningKey::from_bytes(&bytes);
        let verifier = "k".repeat(43);
        let intent = EnrollmentIntent {
            id: id(),
            control_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: signer.verifying_key().to_bytes().into(),
            },
            dataplane_key: NodePublicKey {
                algorithm: KeyAlgorithm::X25519,
                bytes: meshway_model::Digest::from_bytes(Sha256::digest(bytes).into()),
            },
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            hostname: name.into(),
            existing: None,
        };
        let digest = proof::request_digest(&intent).unwrap();
        let request = intent.id;
        let device_proof = |operation: &str| EnrollmentDeviceProof {
            verifier: WireSecret::new(verifier.clone()),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    signer
                        .sign(&enrollment_transcript(request, digest, operation))
                        .to_bytes(),
                ),
            ),
        };
        let _: EnrollmentRequestView = json(
            self.post(
                "/agent-api/v1/enrollment-requests",
                "",
                &CreateEnrollmentRequest {
                    signature: device_proof("create").signature,
                    intent,
                },
            ),
            200,
        )
        .await;
        let _: DeviceAuthorizationView = json(
            self.post(
                &format!("/api/v1/device-authorizations/{request}/approve"),
                user.token.expose(),
                &ApproveEnrollmentRequest {
                    network_id: network,
                },
            ),
            200,
        )
        .await;
        let poll: EnrollmentPollResponse = json(
            self.post(
                &format!("/agent-api/v1/enrollment-requests/{request}/query"),
                "",
                &device_proof("poll"),
            ),
            200,
        )
        .await;
        let enrolled: EnrollmentResponse = json(
            self.post(
                &format!("/agent-api/v1/enrollment-requests/{request}/redeem"),
                "",
                &RedeemEnrollmentRequest {
                    proof: device_proof("redeem"),
                    code: poll.code.unwrap(),
                },
            ),
            200,
        )
        .await;
        let intent = NodeSessionIntent { request_id: id() };
        let challenge: NodeChallenge = json(
            self.post(
                "/agent-api/v1/auth/challenges",
                "",
                &signed_challenge(
                    enrolled.controller_id,
                    NodeChallengeRequest {
                        network_id: network,
                        node_id: enrolled.node_id,
                        key_id: enrolled.keys.control.id,
                        operation: ProofOperation::Session,
                        request_digest: proof::request_digest(&intent).unwrap(),
                    },
                    &signer,
                ),
            ),
            200,
        )
        .await;
        let session: NodeSessionResponse = json(
            self.post(
                "/agent-api/v1/auth/session",
                "",
                &NodeSessionRequest {
                    intent,
                    proof: NodeProof {
                        challenge_id: challenge.id,
                        signature: WireSecret::new(
                            URL_SAFE_NO_PAD
                                .encode(signer.sign(&proof_transcript(&challenge)).to_bytes()),
                        ),
                    },
                },
            ),
            200,
        )
        .await;
        let lease: InstanceLease = json(
            self.post(
                "/agent-api/v1/instances",
                session.token.expose(),
                &AcquireInstanceRequest {
                    instance_id: id(),
                    capabilities: caps(),
                },
            ),
            200,
        )
        .await;
        Node {
            id: enrolled.node_id,
            controller_id: enrolled.controller_id,
            token: session.token,
            signer,
            keys: enrolled.keys,
            instance: InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
        }
    }
    /// Renew the instance and read the Node's current netmap through `sync`.
    pub async fn target(&self, node: &Node) -> meshway_configuration::netmap::CompiledNetmap {
        let _: InstanceLease = json(
            self.post(
                "/agent-api/v1/instances/renew",
                node.token.expose(),
                &node.instance,
            ),
            200,
        )
        .await;
        let response: meshway_protocol::SyncResponse = json(
            self.post(
                "/agent-api/v1/sync",
                node.token.expose(),
                &meshway_protocol::SyncRequest {
                    netmap: None,
                    state: None,
                    server_key_generation: None,
                    traversal_requests: Default::default(),
                    associations: None,
                },
            ),
            200,
        )
        .await;
        match response.netmap {
            meshway_protocol::SyncNetmap::Full { netmap } => *netmap,
            _ => panic!("a sync without a base returns the full netmap"),
        }
    }
    pub async fn configure(
        &self,
        user: &SessionIssuedResponse,
        network: NetworkId,
        edit: impl FnOnce(&mut meshway_configuration::NetworkConfigurationInput),
    ) {
        let source: ConfigurationSourceResponse = json(
            self.get(
                &format!("/api/v1/networks/{network}/configuration/source"),
                user.token.expose(),
            ),
            200,
        )
        .await;
        let mut input = source.configuration.clone();
        edit(&mut input);
        let _: ConfigurationAppliedResponse = json(
            self.post(
                &format!("/api/v1/networks/{network}/configuration"),
                user.token.expose(),
                &ConfigurationRequest::Document {
                    expected_revision: source.revision,
                    request_id: id(),
                    source_jsonc: WireSecret::new(serde_json::to_string(&input).unwrap()),
                },
            ),
            200,
        )
        .await;
    }
    pub async fn stop(self) {
        self.task.abort();
        self.app.shutdown().await.unwrap();
    }
    pub async fn restart(self) -> Self {
        self.task.abort();
        let _ = self.task.await;
        self.app.shutdown().await.unwrap();
        Self::open(self._temp).await
    }
    pub async fn resume(&self, node: &mut Node, network: NetworkId) {
        let intent = NodeSessionIntent { request_id: id() };
        let challenge: NodeChallenge = json(
            self.post(
                "/agent-api/v1/auth/challenges",
                "",
                &signed_challenge(
                    node.controller_id,
                    NodeChallengeRequest {
                        network_id: network,
                        node_id: node.id,
                        key_id: node.keys.control.id,
                        operation: ProofOperation::Session,
                        request_digest: proof::request_digest(&intent).unwrap(),
                    },
                    &node.signer,
                ),
            ),
            200,
        )
        .await;
        let session: NodeSessionResponse = json(
            self.post(
                "/agent-api/v1/auth/session",
                "",
                &NodeSessionRequest {
                    intent,
                    proof: NodeProof {
                        challenge_id: challenge.id,
                        signature: WireSecret::new(
                            URL_SAFE_NO_PAD
                                .encode(node.signer.sign(&proof_transcript(&challenge)).to_bytes()),
                        ),
                    },
                },
            ),
            200,
        )
        .await;
        let lease: InstanceLease = json(
            self.post(
                "/agent-api/v1/instances",
                session.token.expose(),
                &AcquireInstanceRequest {
                    instance_id: id(),
                    capabilities: caps(),
                },
            ),
            200,
        )
        .await;
        node.token = session.token;
        node.instance = InstanceRequest {
            instance_id: lease.instance_id,
            generation: lease.generation,
        };
    }
}
pub struct Node {
    pub id: NodeId,
    controller_id: KeyId,
    pub token: WireSecret,
    pub instance: InstanceRequest,
    signer: SigningKey,
    pub keys: NodeKeyMetadata,
}
pub fn caps() -> BTreeMap<String, meshway_model::state::Capability> {
    use meshway_model::state::*;
    BTreeMap::from([
        (
            "feature:path-observation".into(),
            Capability::Feature { version: 1 },
        ),
        (
            "transport:native_udp".into(),
            Capability::Transport {
                roles: vec![TransportRole::Dial, TransportRole::Listen],
                streams: vec![StreamKind::Udp],
            },
        ),
    ])
}
pub async fn json<T: DeserializeOwned>(request: reqwest::RequestBuilder, status: u16) -> T {
    let response = request.send().await.unwrap();
    let actual = response.status();
    let url = response.url().clone();
    let body = response.text().await.unwrap();
    assert_eq!(actual.as_u16(), status, "{url} {body}");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"))
}
pub async fn status(request: reqwest::RequestBuilder, status: u16) {
    let response = request.send().await.unwrap();
    let actual = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(actual.as_u16(), status, "{body}");
}

pub async fn wait_until(deadline: chrono::DateTime<chrono::Utc>) {
    if let Ok(remaining) = (deadline - chrono::Utc::now()).to_std() {
        tokio::time::sleep(remaining).await;
    }
}

pub fn assert_pre_expiry_budget(task: &meshway_protocol::ObservationTask) {
    let remaining = task.expires_at - chrono::Utc::now();
    assert!(
        remaining
            >= chrono::Duration::seconds(
                meshway_server::observations::OBSERVATION_LEASE_SECONDS / 2,
            ),
        "pre-expiry observation work must fit half the issued lease: {remaining} left",
    );
}

/// Revocation is asynchronous. Bound convergence by the subscription lease,
/// and inspect the Node's actual task list instead of guessing a delay.
pub async fn wait_no_diagnostics(server: &Server, node: &Node) {
    tokio::time::timeout(
        std::time::Duration::from_secs(
            meshway_server::observations::OBSERVATION_LEASE_SECONDS as u64,
        ),
        async {
            loop {
                let tasks: ObservationList =
                    json(server.get("/agent-api/v1/tasks", node.token.expose()), 200).await;
                if tasks.items.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await
    .expect("revocation projection before the original subscription lease");
}
