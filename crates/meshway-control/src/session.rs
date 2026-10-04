use crate::{Control, Deadline, Error, Session, Started, random_id};
use meshway_control_client::ClientError;
use meshway_model::RequestId;
use meshway_protocol::{
    ErrorResponse, NodeChallengeRequest, NodeSessionRequest,
    node::{NodeProof, NodeSessionIntent, ProofOperation, proof_transcript, request_digest},
};
use std::time::{Duration, Instant};

const REFRESH_BEFORE: Duration = Duration::from_secs(60);

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
        let challenge_start = Started::now(&self.clock)?;
        let challenge = self
            .client
            .create_node_challenge(&NodeChallengeRequest {
                network_id: binding.network_id,
                node_id: binding.node_id,
                key_id: binding.keys.control.id,
                operation: ProofOperation::Session,
                request_digest: digest,
            })
            .await;
        let challenge = self.authentication_response(challenge)?;
        if challenge.controller_id != binding.controller_id
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
        let response = self.authentication_response(response)?;
        if response.network_id != binding.network_id
            || response.node_id != binding.node_id
            || response.key_id != binding.keys.control.id
        {
            self.reject_authentication();
            return Err(Error::Contract);
        }
        let deadline = match Deadline::from_response(started, response.expires_at_ms) {
            Ok(deadline) => deadline,
            Err(Error::Expired) => {
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
                return Err(error);
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
        Ok(())
    }

    fn reject_authentication(&mut self) {
        self.fence();
        self.session = None;
        self.authentication = None;
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
        }
        response
    }
}
