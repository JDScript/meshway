//! Durable two-key rotation. Uncertain delivery is recovered, never rolled back
//! by destroying the only private key the Server may already have accepted.
use crate::{Control, Error, identity::Keys, random_id};
use meshway_control_client::ClientError;
use meshway_model::{ChallengeId, RequestId};
use meshway_protocol::{
    EnrollmentResponse, NodeChallenge, NodeChallengeIntent, NodeChallengeRequest, NodeKeyMetadata,
    NodeRotationRequest, SignedNodeChallengeRequest, challenge_request_transcript,
    node::{NodeProof, NodeRotationIntent, ProofOperation, proof_transcript, request_digest},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Rotation {
    keys: Keys,
    intent: NodeRotationIntent,
    allocation: NodeChallengeIntent,
    #[serde(deserialize_with = "Option::deserialize")]
    challenge: Option<NodeChallenge>,
}

impl Rotation {
    pub fn validate(&self, binding: &EnrollmentResponse) -> Result<(), Error> {
        let allocation = &self.allocation;
        let request = &allocation.request;
        if self.keys.control()? != self.intent.control_key
            || self.keys.dataplane()? != self.intent.dataplane_key
            || self.intent.control_key == binding.keys.control.public_key
            || self.intent.dataplane_key == binding.keys.dataplane.public_key
            || allocation.controller_id != binding.controller_id
            || request.network_id != binding.network_id
            || request.node_id != binding.node_id
            || request.key_id != binding.keys.control.id
            || request.operation != ProofOperation::Rotate
            || request.request_digest != request_digest(&self.intent).map_err(|_| Error::Record)?
        {
            return Err(Error::Record);
        }
        if let Some(challenge) = &self.challenge {
            self.validate_challenge(challenge, binding)
                .map_err(|_| Error::Record)?;
        }
        Ok(())
    }

    fn validate_challenge(
        &self,
        challenge: &NodeChallenge,
        binding: &EnrollmentResponse,
    ) -> Result<(), Error> {
        let allocation = &self.allocation;
        if challenge.id != allocation.id
            || challenge.controller_id != allocation.controller_id
            || challenge.network_id != allocation.request.network_id
            || challenge.node_id != allocation.request.node_id
            || challenge.key_id != allocation.request.key_id
            || challenge.key_generation != binding.keys.control.generation
            || challenge.operation != ProofOperation::Rotate
            || challenge.request_digest != allocation.request.request_digest
            || challenge.expires_at_ms != allocation.expires_at_ms
        {
            return Err(Error::Contract);
        }
        Ok(())
    }

    fn request(&self, old: &Keys) -> Result<NodeRotationRequest, Error> {
        let challenge = self.challenge.as_ref().ok_or(Error::Lifecycle)?;
        let transcript = proof_transcript(challenge);
        Ok(NodeRotationRequest {
            intent: self.intent.clone(),
            old_proof: NodeProof {
                challenge_id: challenge.id,
                signature: old.sign(&transcript)?,
            },
            new_signature: self.keys.sign(&transcript)?,
        })
    }
}

impl Control {
    pub fn rotation_pending(&self) -> bool {
        self.record.rotation.is_some()
    }

    /// Explicit operator rotation, or recovery of a previously accepted intent.
    /// No automatic key-expiry policy is inferred from short session deadlines.
    pub async fn rotate_keys(&mut self) -> Result<NodeKeyMetadata, Error> {
        let request_id = match &self.record.rotation {
            Some(rotation) => rotation.intent.request_id,
            None => RequestId::new(random_id()?).map_err(|_| Error::Cryptography)?,
        };
        self.rotate_keys_with_request(request_id).await
    }

    /// Host request identity is durable with the key transition. Retrying the
    /// last completed operation returns its bound metadata, never another key.
    pub async fn rotate_keys_with_request(
        &mut self,
        request_id: RequestId,
    ) -> Result<NodeKeyMetadata, Error> {
        self.ready()?;
        let binding = self.record.binding.clone().ok_or(Error::Lifecycle)?;
        if self.record.leave.is_some() {
            return Err(Error::Pending);
        }
        if self
            .record
            .rotation
            .as_ref()
            .is_some_and(|rotation| rotation.intent.request_id != request_id)
        {
            return Err(Error::Pending);
        }
        if self.record.completed_rotation == Some(request_id) {
            return Ok(binding.keys);
        }
        if self.record.rotation.is_none() {
            let keys = Keys::generate()?;
            let intent = NodeRotationIntent {
                request_id,
                control_key: keys.control()?,
                dataplane_key: keys.dataplane()?,
            };
            let allocation = NodeChallengeIntent {
                id: ChallengeId::new(random_id()?).map_err(|_| Error::Cryptography)?,
                controller_id: binding.controller_id,
                request: NodeChallengeRequest {
                    network_id: binding.network_id,
                    node_id: binding.node_id,
                    key_id: binding.keys.control.id,
                    operation: ProofOperation::Rotate,
                    request_digest: request_digest(&intent).map_err(|_| Error::Record)?,
                },
                expires_at_ms: self
                    .clock
                    .now()?
                    .checked_add(59_000)
                    .ok_or(Error::Expired)?,
            };
            self.record.rotation = Some(Rotation {
                keys,
                intent,
                allocation,
                challenge: None,
            });
            self.persist().await?;
        }
        if self
            .record
            .rotation
            .as_ref()
            .ok_or(Error::Record)?
            .challenge
            .is_none()
        {
            self.prepare_rotation(&binding).await?;
        }
        let pending = self.record.rotation.as_ref().ok_or(Error::Record)?;
        let request = pending.request(self.record.keys.as_ref().ok_or(Error::Record)?)?;
        let expires = pending.allocation.expires_at_ms;
        // A submitted rotation can retire all current execution. Stop old local
        // authority before sending, including when delivery becomes uncertain.
        self.fence();
        self.revoke_netmap();
        self.session = None;
        self.authentication = None;
        self.challenge_intent = None;
        let response = self.client.rotate_node_keys(&request).await;
        let keys = match response {
            Ok(keys) => keys,
            Err(error) => {
                if self.clock.now()? >= expires
                    && matches!(
                        &error,
                        ClientError::Http {
                            status: 400 | 401 | 409,
                            ..
                        }
                    )
                {
                    // A rejected expired request can no longer commit later.
                    // Only a successful fresh old-key proof permits rollback.
                    // Failure of that proof retains BOTH keys for recovery.
                    if self.refresh_authentication().await.is_ok() {
                        self.record.rotation = None;
                        self.persist().await?;
                        return Err(Error::Expired);
                    }
                }
                return Err(Error::Server(error));
            }
        };
        let pending = self.record.rotation.as_ref().ok_or(Error::Record)?;
        let generation = binding
            .keys
            .control
            .generation
            .checked_next()
            .map_err(|_| Error::Contract)?;
        if keys.control.public_key != pending.intent.control_key
            || keys.dataplane.public_key != pending.intent.dataplane_key
            || keys.control.generation != generation
            || keys.dataplane.generation != generation
            || keys.control.id == binding.keys.control.id
            || keys.dataplane.id == binding.keys.dataplane.id
            || keys.control.id == keys.dataplane.id
        {
            return Err(Error::Contract);
        }
        let pending = self.record.rotation.take().ok_or(Error::Record)?;
        self.record.keys = Some(pending.keys);
        self.record.binding = Some(EnrollmentResponse {
            keys: keys.clone(),
            ..binding
        });
        self.record.completed_rotation = Some(request_id);
        // The old content is not even a delta base for the replacement keys.
        self.netmap = Default::default();
        // One atomic record replacement publishes keys and binding together.
        // Failed/cancelled writes require reopen; the disk has pending or new.
        self.persist().await?;
        Ok(keys)
    }

    async fn prepare_rotation(&mut self, binding: &EnrollmentResponse) -> Result<(), Error> {
        let now = self.clock.now()?;
        let pending = self.record.rotation.as_mut().ok_or(Error::Record)?;
        if pending.allocation.expires_at_ms <= now {
            pending.allocation.id =
                ChallengeId::new(random_id()?).map_err(|_| Error::Cryptography)?;
            pending.allocation.expires_at_ms = now.checked_add(59_000).ok_or(Error::Expired)?;
            self.persist().await?;
        }
        let pending = self.record.rotation.as_ref().ok_or(Error::Record)?;
        let request = SignedNodeChallengeRequest {
            intent: pending.allocation.clone(),
            signature: self
                .record
                .keys
                .as_ref()
                .ok_or(Error::Record)?
                .sign(&challenge_request_transcript(&pending.allocation))?,
        };
        let challenge = self.client.create_node_challenge(&request).await?;
        let pending = self.record.rotation.as_mut().ok_or(Error::Record)?;
        pending.validate_challenge(&challenge, binding)?;
        pending.challenge = Some(challenge);
        // Persist the exact recovery challenge BEFORE the first rotate request.
        self.persist().await
    }
}
