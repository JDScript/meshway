use crate::{Control, Deadline, Error, Session, Started, random_id};
use meshway_control_client::ClientError;
use meshway_model::{ChallengeId, RequestId};
use meshway_protocol::{
    ErrorResponse, NodeChallengeIntent, NodeChallengeRequest, NodeSessionRequest,
    SignedNodeChallengeRequest, challenge_request_transcript,
    node::{NodeProof, NodeSessionIntent, ProofOperation, proof_transcript, request_digest},
};
use std::time::{Duration, Instant};

const REFRESH_BEFORE: Duration = Duration::from_secs(60);
// The Server accepts at most 60 seconds from its own UTC observation. The
// monotonic high-water projection may round ahead of wall time by a millisecond;
// do not fill that entire signed request budget on a fast local connection.
// This is a requested proof lifetime, not a session/instance/execution lease.
const CHALLENGE_LIFETIME_MS: u64 = 59_000;

impl Control {
    /// Reuse a live session until either its UTC high-water or monotonic
    /// remaining lifetime reaches 60 seconds, then refresh it proactively.
    pub async fn authenticate(&mut self) -> Result<(), Error> {
        self.ready()?;
        if self.authentication.is_none()
            && let Some(session) = &mut self.session
            && session.deadline.valid()
            && Duration::from_millis(
                session
                    .deadline
                    .expires_at_ms()
                    .saturating_sub(self.clock.now()?),
            ) > REFRESH_BEFORE
            && session
                .deadline
                .expires_at()
                .checked_duration_since(Instant::now())
                .is_some_and(|remaining| remaining > REFRESH_BEFORE)
        {
            session.active = true;
            return Ok(());
        }
        self.refresh_authentication().await
    }

    /// Obtain a fresh held-key session without extending instance or execution
    /// leases. A still-live session keeps its authority during the proof request;
    /// old snapshots retain their original deadlines after successful handoff.
    /// Cancellation and uncertain delivery retain the same session intent.
    pub async fn refresh_authentication(&mut self) -> Result<(), Error> {
        self.ready()?;
        if self
            .session
            .as_ref()
            .is_none_or(|session| !session.active || !session.deadline.valid())
        {
            self.session = None;
            self.fence();
        } else if self
            .instance
            .as_ref()
            .is_some_and(|instance| !instance.deadline.valid())
        {
            self.fence();
        }
        let binding = self.record.binding.clone().ok_or(Error::Lifecycle)?;
        if self.authentication.is_none() {
            self.authentication = Some(NodeSessionIntent {
                request_id: RequestId::new(random_id()?).map_err(|_| Error::Cryptography)?,
            });
        }
        let intent = self.authentication.clone().ok_or(Error::Lifecycle)?;
        let digest = request_digest(&intent).map_err(|_| Error::Record)?;
        let request = NodeChallengeRequest {
            network_id: binding.network_id,
            node_id: binding.node_id,
            key_id: binding.keys.control.id,
            operation: ProofOperation::Session,
            request_digest: digest,
        };
        let now = self.clock.now()?;
        if self.challenge_intent.as_ref().is_none_or(|pending| {
            pending.controller_id != binding.controller_id
                || pending.request != request
                || pending.expires_at_ms <= now
        }) {
            self.challenge_intent = Some(NodeChallengeIntent {
                id: ChallengeId::new(random_id()?).map_err(|_| Error::Cryptography)?,
                controller_id: binding.controller_id,
                request,
                expires_at_ms: now
                    .checked_add(CHALLENGE_LIFETIME_MS)
                    .ok_or(Error::Expired)?,
            });
        }
        // Retain this exact absolute-expiry intent across uncertain HTTP delivery
        // and cancellation. Retrying allocation must not spend another quota slot.
        let challenge_intent = self.challenge_intent.clone().ok_or(Error::Lifecycle)?;
        let signature = self
            .record
            .keys
            .as_ref()
            .ok_or(Error::Record)?
            .sign(&challenge_request_transcript(&challenge_intent))?;
        let challenge_start = Started::now(&self.clock)?;
        let challenge = self
            .client
            .create_node_challenge(&SignedNodeChallengeRequest {
                intent: challenge_intent.clone(),
                signature,
            })
            .await;
        let challenge = self.proof_response(challenge, challenge_intent.expires_at_ms)?;
        // Session-response recovery requires a NEW challenge and the SAME
        // session intent; a consumed challenge is never a reusable proof.
        self.challenge_intent = None;
        if challenge.id != challenge_intent.id
            || challenge.expires_at_ms != challenge_intent.expires_at_ms
            || challenge.controller_id != binding.controller_id
            || challenge.network_id != binding.network_id
            || challenge.node_id != binding.node_id
            || challenge.key_id != binding.keys.control.id
            || challenge.key_generation != binding.keys.control.generation
            || challenge.operation != ProofOperation::Session
            || challenge.request_digest != digest
        {
            self.reject_authentication();
            return Err(Error::Contract);
        }
        Deadline::from_response(challenge_start, challenge.expires_at_ms)?;
        let signature = self
            .record
            .keys
            .as_ref()
            .ok_or(Error::Record)?
            .sign(&proof_transcript(&challenge))?;
        let started = Started::now(&self.clock)?;
        let response = self
            .client
            .create_node_session(&NodeSessionRequest {
                intent,
                proof: NodeProof {
                    challenge_id: challenge.id,
                    signature,
                },
            })
            .await;
        let response = self.proof_response(response, challenge.expires_at_ms)?;
        if response.network_id != binding.network_id
            || response.node_id != binding.node_id
            || response.key_id != binding.keys.control.id
        {
            self.reject_authentication();
            return Err(Error::Contract);
        }
        let deadline = match Deadline::from_response(started, response.expires_at_ms) {
            Ok(deadline) => deadline,
            Err(meshway_authority::Error::Expired) => {
                // An idempotent replay retains its original Server expiry. Once
                // proven expired it cannot be recovered by another challenge;
                // allow the next call to issue a genuinely new session intent.
                self.authentication = None;
                if self
                    .session
                    .as_ref()
                    .is_none_or(|session| !session.deadline.valid())
                {
                    self.session = None;
                    self.fence();
                }
                return Err(Error::Expired);
            }
            Err(error) => {
                self.reject_authentication();
                return Err(error.into());
            }
        };
        // The exclusive Control borrow keeps identity/instance ownership fixed
        // throughout this request. Time can still expire either old grant while
        // HTTP is in flight; replacing a token must never revive that context.
        if self
            .session
            .as_ref()
            .is_none_or(|session| !session.active || !session.deadline.valid())
            || self
                .instance
                .as_ref()
                .is_some_and(|instance| !instance.deadline.valid())
        {
            self.fence();
        }
        self.session = Some(Session {
            response,
            deadline,
            active: true,
        });
        self.authentication = None;
        self.challenge_intent = None;
        Ok(())
    }

    fn reject_authentication(&mut self) {
        self.fence();
        self.session = None;
        self.authentication = None;
        self.challenge_intent = None;
    }

    fn proof_response<T>(
        &mut self,
        response: Result<T, ClientError<ErrorResponse>>,
        proof_expires_at_ms: u64,
    ) -> Result<T, Error> {
        // A proof can expire in transit. Its rejection cannot decide whether a
        // fresh proof of the same pinned identity would succeed. Never retry a
        // fresh proof's credential rejection, or accept any rejected response.
        let expired_rejection = matches!(
            &response,
            Err(ClientError::Http { status: 401, .. } | ClientError::Network { status: Some(401) })
        ) && self.clock.now()? >= proof_expires_at_ms;
        let result = self.authentication_response(response);
        if expired_rejection {
            self.reject_authentication();
            Err(Error::Expired)
        } else {
            result
        }
    }

    fn authentication_response<T>(
        &mut self,
        response: Result<T, ClientError<ErrorResponse>>,
    ) -> Result<T, Error> {
        // A received rejection status remains authoritative even if reading
        // its response body fails. A transport failure without it is retryable.
        let rejected_transport = matches!(
            &response,
            Err(ClientError::Network {
                status: Some(401 | 403 | 404 | 409),
            })
        );
        let definitive = matches!(
            &response,
            Err(ClientError::Http {
                status: 401 | 403 | 404 | 409,
                ..
            } | ClientError::Decode { .. }
                | ClientError::BodyLimit { .. })
        );
        let response = self.registered(response);
        if rejected_transport {
            self.reject_authentication();
        } else if definitive {
            // registered fences all old authority; an expired/rejected intent
            // must not trap later authentication in an endless conflict replay.
            self.authentication = None;
            self.challenge_intent = None;
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post,
    };
    use meshway_model::{BoundNodeKey, Version};
    use meshway_protocol::{EnrollmentResponse, NodeChallenge, node::NodeKeyMetadata};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    };
    use tokio::sync::Notify;

    #[derive(Clone, Default)]
    struct Wire {
        mode: Arc<AtomicU8>,
        challenges: Arc<Mutex<Vec<NodeChallengeIntent>>>,
        sessions: Arc<Mutex<Vec<NodeSessionIntent>>>,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    async fn allocate(
        State(wire): State<Wire>,
        Json(request): Json<SignedNodeChallengeRequest>,
    ) -> axum::response::Response {
        wire.challenges.lock().unwrap().push(request.intent.clone());
        let mode = wire.mode.load(Ordering::SeqCst);
        if mode == 2 {
            wire.entered.notify_one();
            wire.release.notified().await;
        }
        if mode != 1 {
            // Delivery fault, not an authoritative authentication rejection.
            return StatusCode::BAD_GATEWAY.into_response();
        }
        let intent = request.intent;
        Json(NodeChallenge {
            id: intent.id,
            controller_id: intent.controller_id,
            network_id: intent.request.network_id,
            node_id: intent.request.node_id,
            key_id: intent.request.key_id,
            key_generation: Version::MIN,
            nonce: [9; 32].into(),
            operation: intent.request.operation,
            request_digest: intent.request.request_digest,
            expires_at_ms: intent.expires_at_ms,
        })
        .into_response()
    }

    async fn issue(
        State(wire): State<Wire>,
        Json(request): Json<NodeSessionRequest>,
    ) -> StatusCode {
        wire.sessions.lock().unwrap().push(request.intent);
        StatusCode::BAD_GATEWAY
    }

    struct Fixture {
        control: Control,
        wire: Wire,
        server: tokio::task::JoinHandle<()>,
        _directory: tempfile::TempDir,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.wire.release.notify_one();
            self.server.abort();
        }
    }
    impl Fixture {
        async fn close(mut self) {
            self.control.stop().await.unwrap();
            self.wire.release.notify_one();
            self.server.abort();
            let _ = (&mut self.server).await;
        }

        async fn open() -> Self {
            let wire = Wire::default();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let router = Router::new()
                .route("/agent-api/v1/auth/challenges", post(allocate))
                .route("/agent-api/v1/auth/session", post(issue))
                .with_state(wire.clone());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let directory = tempfile::tempdir().unwrap();
            let mut control = Control::open(
                &directory.path().canonicalize().unwrap().join("identity"),
                &origin,
                meshway_control_client::http_client_builder()
                    .unwrap()
                    .no_proxy(),
            )
            .unwrap();
            let keys = crate::identity::Keys::generate().unwrap();
            let binding = EnrollmentResponse {
                controller_id: random_id().unwrap().try_into().unwrap(),
                network_id: random_id().unwrap().try_into().unwrap(),
                node_id: random_id().unwrap().try_into().unwrap(),
                keys: NodeKeyMetadata {
                    control: BoundNodeKey {
                        id: random_id().unwrap().try_into().unwrap(),
                        generation: Version::MIN,
                        public_key: keys.control().unwrap(),
                    },
                    dataplane: BoundNodeKey {
                        id: random_id().unwrap().try_into().unwrap(),
                        generation: Version::MIN,
                        public_key: keys.dataplane().unwrap(),
                    },
                },
                server_signing_key: None,
            };
            // Only the Control retry state machine is under test here. Real
            // registration/signature/quota admission is covered by Server tests.
            control.record.keys = Some(keys);
            control.record.binding = Some(binding);
            Self {
                control,
                wire,
                server,
                _directory: directory,
            }
        }
    }

    #[tokio::test]
    async fn uncertain_allocation_reuses_id_but_uncertain_session_uses_fresh_proof() {
        let mut fixture = Fixture::open().await;
        assert!(fixture.control.authenticate().await.is_err());
        let allocation = fixture.control.challenge_intent.clone().unwrap();
        let session = fixture.control.authentication.clone().unwrap();
        assert!(fixture.control.authenticate().await.is_err());
        assert_eq!(fixture.control.challenge_intent.as_ref(), Some(&allocation));
        assert_eq!(
            fixture.wire.challenges.lock().unwrap().as_slice(),
            &[allocation.clone(), allocation.clone()]
        );

        fixture.wire.mode.store(1, Ordering::SeqCst);
        assert!(fixture.control.authenticate().await.is_err());
        assert!(fixture.control.challenge_intent.is_none());
        assert_eq!(fixture.control.authentication.as_ref(), Some(&session));
        assert!(fixture.control.authenticate().await.is_err());
        {
            let requests = fixture.wire.challenges.lock().unwrap();
            assert_eq!(requests[2], allocation);
            assert_ne!(requests[3].id, allocation.id);
            assert_eq!(requests[3].request, allocation.request);
            assert_eq!(
                fixture.wire.sessions.lock().unwrap().as_slice(),
                &[session.clone(), session]
            );
        }
        fixture.close().await;
    }

    #[tokio::test]
    async fn cancellation_retains_allocation_and_expiry_replaces_only_challenge_intent() {
        let mut fixture = Fixture::open().await;
        fixture.wire.mode.store(2, Ordering::SeqCst);
        {
            let authenticate = fixture.control.authenticate();
            tokio::pin!(authenticate);
            tokio::select! {
                result = &mut authenticate => panic!("expected held request: {result:?}"),
                result = tokio::time::timeout(Duration::from_secs(2), fixture.wire.entered.notified()) => result.unwrap(),
            }
        }
        fixture.wire.release.notify_one();
        let allocation = fixture.control.challenge_intent.clone().unwrap();
        let session = fixture.control.authentication.clone().unwrap();
        fixture.wire.mode.store(0, Ordering::SeqCst);
        assert!(fixture.control.authenticate().await.is_err());
        assert_eq!(fixture.control.challenge_intent.as_ref(), Some(&allocation));
        fixture
            .control
            .challenge_intent
            .as_mut()
            .unwrap()
            .expires_at_ms = 0;
        assert!(fixture.control.authenticate().await.is_err());
        let replacement = fixture.control.challenge_intent.as_ref().unwrap();
        assert_ne!(replacement.id, allocation.id);
        assert_eq!(replacement.request, allocation.request);
        assert_eq!(fixture.control.authentication.as_ref(), Some(&session));
        fixture.control.stop().await.unwrap();
        assert!(fixture.control.challenge_intent.is_none());
        assert!(fixture.control.authentication.is_none());
        fixture.close().await;
    }

    #[tokio::test]
    async fn definitive_rejection_discards_pending_allocation_owner() {
        let mut fixture = Fixture::open().await;
        assert!(fixture.control.authenticate().await.is_err());
        assert!(fixture.control.challenge_intent.is_some());
        let result = fixture
            .control
            .authentication_response::<()>(Err(ClientError::Network { status: Some(401) }));
        assert!(result.is_err());
        assert!(fixture.control.challenge_intent.is_none());
        assert!(fixture.control.authentication.is_none());
        fixture.close().await;
    }

    #[tokio::test]
    async fn expired_proof_rejection_requires_a_new_proof_but_fresh_rejection_is_final() {
        let mut fixture = Fixture::open().await;
        assert!(fixture.control.authenticate().await.is_err());
        let pending = fixture.control.challenge_intent.as_ref().unwrap();
        let now = fixture.control.clock.now().unwrap();
        assert!(pending.expires_at_ms.saturating_sub(now) < 60_000);
        assert!(matches!(
            fixture.control.proof_response::<()>(
                Err(ClientError::Http {
                    status: 401,
                    problem: None
                }),
                0
            ),
            Err(Error::Expired)
        ));
        assert!(fixture.control.challenge_intent.is_none());
        assert!(!fixture.control.status().authenticated);
        assert!(matches!(
            fixture.control.proof_response::<()>(
                Err(ClientError::Http {
                    status: 401,
                    problem: None
                }),
                now + CHALLENGE_LIFETIME_MS
            ),
            Err(Error::Server(ClientError::Http { status: 401, .. }))
        ));
        assert!(!fixture.control.status().authenticated);
        fixture.close().await;
    }
}
