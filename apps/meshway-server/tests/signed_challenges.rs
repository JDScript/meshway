//! Real enrollment, Router admission, held-key proofs and durable challenge rows.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use meshway_model::*;
use meshway_protocol::{
    NodeChallengeIntent, SignedNodeChallengeRequest, WireSecret, challenge_request_transcript,
    enrollment::*, node::*,
};
use meshway_server::{App, Settings, accounts::Secret};
use sha2::{Digest as _, Sha256};

fn id<T: TryFrom<uuid::Uuid>>() -> T
where
    T::Error: std::fmt::Debug,
{
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).unwrap();
    T::try_from(uuid::Uuid::from_bytes(bytes)).unwrap()
}

fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().try_into().unwrap()
}

fn sign(signer: &SigningKey, intent: NodeChallengeIntent) -> SignedNodeChallengeRequest {
    let signature = signer.sign(&challenge_request_transcript(&intent));
    SignedNodeChallengeRequest {
        intent,
        signature: WireSecret::new(URL_SAFE_NO_PAD.encode(signature.to_bytes())),
    }
}

struct Serving(tokio::task::JoinHandle<()>);
impl Drop for Serving {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Fixture {
    app: App,
    binding: EnrollmentResponse,
    signer: SigningKey,
    origin: String,
    client: reqwest::Client,
    serving: Serving,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn open() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let app = App::open(Settings::new(
            directory.path().canonicalize().unwrap().join("data"),
        ))
        .await
        .unwrap();
        let owner = app
            .accounts()
            .register(
                "challenge-owner@example.test".into(),
                Secret::new("signed challenge fixture password".into()),
                id(),
            )
            .await
            .unwrap();
        let network = app
            .networks()
            .create(owner.token.expose(), "signed challenges".into(), id())
            .await
            .unwrap();
        let signer = SigningKey::from_bytes(&[83; 32]);
        let verifier = "v".repeat(43);
        let intent = EnrollmentIntent {
            id: id(),
            existing: None,
            control_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: signer.verifying_key().to_bytes().into(),
            },
            dataplane_key: NodePublicKey {
                algorithm: KeyAlgorithm::X25519,
                bytes: [27; 32].into(),
            },
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            hostname: "signed-challenge-node".into(),
        };
        let enrollment_id = intent.id;
        let digest = request_digest(&intent).unwrap();
        let proof = |operation: &str| EnrollmentDeviceProof {
            verifier: WireSecret::new(verifier.clone()),
            signature: WireSecret::new(
                URL_SAFE_NO_PAD.encode(
                    signer
                        .sign(&enrollment_transcript(enrollment_id, digest, operation))
                        .to_bytes(),
                ),
            ),
        };
        app.enrollment()
            .create_request(CreateEnrollmentRequest {
                signature: proof("create").signature,
                intent,
            })
            .await
            .unwrap();
        app.enrollment()
            .approve(
                owner.token.expose(),
                enrollment_id,
                ApproveEnrollmentRequest {
                    network_id: network.id,
                },
            )
            .await
            .unwrap();
        let polled = app
            .enrollment()
            .poll(enrollment_id, proof("poll"))
            .await
            .unwrap();
        let binding = app
            .enrollment()
            .redeem(
                enrollment_id,
                RedeemEnrollmentRequest {
                    proof: proof("redeem"),
                    code: polled.code.unwrap(),
                },
            )
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let router = meshway_server::http::router(&app);
        let serving = Serving(tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        }));
        Self {
            app,
            binding,
            signer,
            origin,
            serving,
            client: reqwest::Client::builder()
                .tls_backend_preconfigured(
                    meshway_transport_stream::tls::platform_client_config().unwrap(),
                )
                .no_proxy()
                .build()
                .unwrap(),
            _directory: directory,
        }
    }

    fn intent(&self, session: &NodeSessionIntent) -> NodeChallengeIntent {
        NodeChallengeIntent {
            id: id(),
            controller_id: self.binding.controller_id,
            request: NodeChallengeRequest {
                network_id: self.binding.network_id,
                node_id: self.binding.node_id,
                key_id: self.binding.keys.control.id,
                operation: ProofOperation::Session,
                request_digest: request_digest(session).unwrap(),
            },
            expires_at_ms: now() + 30_000,
        }
    }

    async fn post(&self, path: &str, body: &impl serde::Serialize) -> reqwest::Response {
        self.client
            .post(format!("{}/agent-api/v1/auth/{path}", self.origin))
            .json(body)
            .send()
            .await
            .unwrap()
    }

    async fn challenge(&self, intent: &NodeChallengeIntent) -> NodeChallenge {
        let response = self
            .post("challenges", &sign(&self.signer, intent.clone()))
            .await;
        assert_eq!(response.status(), 200);
        response.json().await.unwrap()
    }

    fn session_request(
        &self,
        intent: &NodeSessionIntent,
        challenge: &NodeChallenge,
    ) -> NodeSessionRequest {
        NodeSessionRequest {
            intent: intent.clone(),
            proof: NodeProof {
                challenge_id: challenge.id,
                signature: WireSecret::new(
                    URL_SAFE_NO_PAD
                        .encode(self.signer.sign(&proof_transcript(challenge)).to_bytes()),
                ),
            },
        }
    }

    async fn close(mut self) {
        self.serving.0.abort();
        let _ = (&mut self.serving.0).await;
        self.app.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn anonymous_requests_cannot_spend_node_quota_and_signed_retries_bypass_full_quota() {
    let fixture = Fixture::open().await;
    let session = NodeSessionIntent { request_id: id() };
    let intent = fixture.intent(&session);
    let stranger = SigningKey::from_bytes(&[84; 32]);
    for _ in 0..12 {
        assert!(
            fixture
                .post("challenges", &intent.request)
                .await
                .status()
                .is_client_error()
        );
        assert_eq!(
            fixture
                .post("challenges", &sign(&stranger, intent.clone()))
                .await
                .status(),
            401
        );
    }
    let first = fixture.challenge(&intent).await;
    for _ in 1..8 {
        fixture.challenge(&fixture.intent(&session)).await;
    }
    assert_eq!(
        fixture
            .post(
                "challenges",
                &sign(&fixture.signer, fixture.intent(&session))
            )
            .await
            .status(),
        503
    );
    for _ in 0..12 {
        assert_eq!(fixture.challenge(&intent).await, first);
    }
    fixture.close().await;
}

#[tokio::test]
async fn signed_binding_conflicts_and_consumed_proof_replays_fail_but_fresh_proof_recovers_session()
{
    let fixture = Fixture::open().await;
    let session = NodeSessionIntent { request_id: id() };
    let intent = fixture.intent(&session);
    let first = fixture.challenge(&intent).await;
    assert_eq!(first.id, intent.id);
    assert_eq!(first.expires_at_ms, intent.expires_at_ms);
    for field in 0..8 {
        let mut changed = intent.clone();
        match field {
            0 => changed.controller_id = id(),
            1 => changed.request.network_id = id(),
            2 => changed.request.node_id = id(),
            3 => changed.request.key_id = id(),
            4 => changed.request.operation = ProofOperation::Rotate,
            5 => changed.request.request_digest = [99; 32].into(),
            6 => changed.expires_at_ms -= 1,
            7 => changed.id = id(),
            _ => unreachable!(),
        }
        // A captured signature cannot be moved to any other binding, even an
        // otherwise valid fresh client ID or a shorter requested lifetime.
        let mut tampered = sign(&fixture.signer, intent.clone());
        tampered.intent = changed.clone();
        assert_eq!(fixture.post("challenges", &tampered).await.status(), 401);
        if (4..=6).contains(&field) {
            assert_eq!(
                fixture
                    .post("challenges", &sign(&fixture.signer, changed))
                    .await
                    .status(),
                409
            );
        }
    }
    let proof = fixture.session_request(&session, &first);
    let response = fixture.post("session", &proof).await;
    assert_eq!(response.status(), 200);
    let original: NodeSessionResponse = response.json().await.unwrap();
    assert_eq!(fixture.challenge(&intent).await, first);
    assert_eq!(fixture.post("session", &proof).await.status(), 401);

    // Recovery is keyed by a new signed allocation ID, not request_digest alone.
    let fresh_intent = fixture.intent(&session);
    let fresh = fixture.challenge(&fresh_intent).await;
    assert_ne!(fresh.id, first.id);
    assert_ne!(fresh.nonce, first.nonce);
    let fresh_proof = fixture.session_request(&session, &fresh);
    let response = fixture.post("session", &fresh_proof).await;
    assert_eq!(response.status(), 200);
    let recovered: NodeSessionResponse = response.json().await.unwrap();
    assert_eq!(recovered.token.expose(), original.token.expose());
    assert_eq!(recovered.expires_at_ms, original.expires_at_ms);
    assert_eq!(fixture.challenge(&fresh_intent).await, fresh);
    assert_eq!(fixture.post("session", &fresh_proof).await.status(), 401);
    // Fresh recovery retains a bounded signed-ID tombstone, not another session
    // anchor. Exhaustion cannot defeat idempotent reads or revive used proofs.
    for _ in 2..16 {
        let recovery = fixture.challenge(&fixture.intent(&session)).await;
        let response = fixture
            .post("session", &fixture.session_request(&session, &recovery))
            .await;
        assert_eq!(response.status(), 200);
        let recovered: NodeSessionResponse = response.json().await.unwrap();
        assert_eq!(recovered.token.expose(), original.token.expose());
    }
    assert_eq!(
        fixture
            .post(
                "challenges",
                &sign(&fixture.signer, fixture.intent(&session))
            )
            .await
            .status(),
        503
    );
    assert_eq!(fixture.challenge(&intent).await, first);
    fixture.close().await;
}

#[tokio::test]
async fn expired_signed_allocation_cannot_be_recreated_after_challenge_cleanup() {
    let fixture = Fixture::open().await;
    let session = NodeSessionIntent { request_id: id() };
    for expiry in [now().saturating_sub(1), now() + 120_000] {
        let mut intent = fixture.intent(&session);
        intent.expires_at_ms = expiry;
        let signed = sign(&fixture.signer, intent);
        assert_eq!(fixture.post("challenges", &signed).await.status(), 401);
    }
    let mut overflow = fixture.intent(&session);
    overflow.expires_at_ms = u64::MAX;
    assert!(matches!(
        fixture
            .app
            .execution()
            .challenge(sign(&fixture.signer, overflow))
            .await,
        Err(meshway_server::error::Error::InvalidCredentials)
    ));
    let mut short = fixture.intent(&session);
    short.expires_at_ms = now() + 500;
    let original = fixture.challenge(&short).await;
    tokio::time::sleep(std::time::Duration::from_millis(
        short.expires_at_ms.saturating_sub(now()) + 1,
    ))
    .await;
    // Successful fresh allocation runs cleanup of the expired unconsumed row.
    fixture.challenge(&fixture.intent(&session)).await;
    assert_eq!(
        fixture
            .post("challenges", &sign(&fixture.signer, short))
            .await
            .status(),
        401
    );
    assert_eq!(
        fixture
            .post("session", &fixture.session_request(&session, &original))
            .await
            .status(),
        401
    );
    fixture.close().await;
}
