use crate::{Control, Deadline, Error, Session, Started, random_id};
use meshway_model::RequestId;
use meshway_protocol::{
    NodeChallengeRequest, NodeSessionRequest,
    node::{NodeProof, NodeSessionIntent, ProofOperation, proof_transcript, request_digest},
};

impl Control {
    pub async fn authenticate(&mut self) -> Result<(), Error> {
        self.ready()?;
        if let Some(session) = &mut self.session
            && session.deadline.valid()
        {
            session.active = true;
            return Ok(());
        }
        self.session = None;
        self.fence();
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        if self.authentication.is_none() {
            self.authentication = Some(NodeSessionIntent {
                request_id: RequestId::new(random_id()?).map_err(|_| Error::Cryptography)?,
            });
        }
        let intent = self.authentication.as_ref().ok_or(Error::Lifecycle)?;
        let digest = request_digest(intent).map_err(|_| Error::Record)?;
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
            .await?;
        if challenge.controller_id != binding.controller_id
            || challenge.network_id != binding.network_id
            || challenge.node_id != binding.node_id
            || challenge.key_id != binding.keys.control.id
            || challenge.key_generation != binding.keys.control.generation
            || challenge.operation != ProofOperation::Session
            || challenge.request_digest != digest
        {
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
                intent: intent.clone(),
                proof: NodeProof {
                    challenge_id: challenge.id,
                    signature,
                },
            })
            .await?;
        if response.network_id != binding.network_id
            || response.node_id != binding.node_id
            || response.key_id != binding.keys.control.id
        {
            return Err(Error::Contract);
        }
        let deadline = Deadline::from_response(started, response.expires_at_ms)?;
        self.session = Some(Session {
            response,
            deadline,
            active: true,
        });
        self.authentication = None;
        Ok(())
    }
}
