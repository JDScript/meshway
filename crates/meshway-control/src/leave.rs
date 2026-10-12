//! Self-service deletion: the Server deletes this Node the way a manager
//! would (permanent revocation, netmap withdrawal), then the identity is
//! forgotten locally. The operator's intent is durable before the request is
//! sent: an unanswered or lost request keeps it, and either the Server's
//! confirmation or its later rejection of this identity completes it.
use crate::{Control, Error, identity_rejected, netmap};
use meshway_control_client::ClientError;
use meshway_model::RequestId;
use meshway_protocol::{ErrorResponse, NodeLeaveRequest, NodeLeaveResponse};
use std::sync::Arc;

/// How a leave completed. Either way the identity is forgotten locally.
#[derive(Clone, Debug)]
pub enum Left {
    /// The Server confirmed this request's deletion.
    Confirmed(NodeLeaveResponse),
    /// The Server rejected this identity while the leave was pending: an
    /// earlier attempt (whose answer was lost) or a manager deleted it.
    IdentityRejected,
}

/// A decoded answer that refuses this leave for a reason other than the
/// identity (permission, configuration): resending cannot succeed, so the
/// intent is withdrawn. A conflict or an unavailable Server keeps it.
fn refused(error: &ClientError<ErrorResponse>) -> bool {
    matches!(error, ClientError::Http { status: status @ (403 | 422), problem: Some(body) }
        if body.status == *status && !identity_rejected(error))
}

impl Control {
    /// The pending leave intent, if any.
    pub fn leaving(&self) -> Option<RequestId> {
        self.record.leave
    }

    /// Persist the leave intent (an existing one keeps its request ID), then
    /// authenticate and send it. Only a confirmation or an identity rejection
    /// clears local records; a definitive refusal withdraws the intent; any
    /// other failure keeps it for the next call, also after reopening.
    pub async fn leave(&mut self, request_id: RequestId) -> Result<Left, Error> {
        self.ready()?;
        if self.record.rotation.is_some() || self.record.enrollment.is_some() {
            return Err(Error::Pending);
        }
        let binding = self.record.binding.clone().ok_or(Error::Lifecycle)?;
        let request_id = match self.record.leave {
            Some(pending) => pending,
            None => {
                self.record.leave = Some(request_id);
                self.persist().await?;
                request_id
            }
        };
        let request = NodeLeaveRequest {
            request_id,
            node_id: binding.node_id,
        };
        // A 401 means the session ended, possibly with the identity: one
        // fresh authentication tells which.
        for attempt in 0..2 {
            if !self.authentication_current()?
                && let Err(error) = self.refresh_authentication().await
            {
                return self.left_unless_rejected(error).await;
            }
            let token = zeroize::Zeroizing::new(self.token()?.to_owned());
            let response = self.client.leave_node(&request, &token).await;
            let expired = matches!(
                &response,
                Err(ClientError::Http { status: 401, .. }
                    | ClientError::Network { status: Some(401) })
            );
            match self.registered(response) {
                Ok(response) => {
                    if response.network_id != binding.network_id
                        || response.node_id != binding.node_id
                    {
                        return Err(Error::Contract);
                    }
                    self.forget().await?;
                    return Ok(Left::Confirmed(response));
                }
                Err(_) if expired && attempt == 0 => continue,
                Err(error) => return self.left_unless_rejected(error).await,
            }
        }
        unreachable!("the second attempt returns")
    }

    async fn left_unless_rejected(&mut self, error: Error) -> Result<Left, Error> {
        match &error {
            Error::Server(server) if identity_rejected(server) => {
                self.forget().await?;
                Ok(Left::IdentityRejected)
            }
            Error::Server(server) if refused(server) => {
                self.record.leave = None;
                self.persist().await?;
                Err(error)
            }
            _ => Err(error),
        }
    }

    /// An explicit new enrollment replaces a pending leave: the held
    /// identity is forgotten without the Server's answer.
    pub async fn abandon_leave(&mut self) -> Result<(), Error> {
        self.ready()?;
        if self.record.leave.is_none() {
            return Err(Error::Lifecycle);
        }
        self.forget().await
    }

    /// Drop every local authority and durable identity of the deleted Node:
    /// the identity record keeps only its origin, and the netmap and peer-fact
    /// caches are removed. The directory stays an initialized, empty identity.
    async fn forget(&mut self) -> Result<(), Error> {
        self.authentication = None;
        self.challenge_intent = None;
        self.fence();
        self.session = None;
        if let Some(write) = self.netmap_write.take() {
            let _ = write.await;
        }
        self.finish_facts_write().await;
        self.finish_floor_write().await;
        self.netmap = netmap::State::default();
        self.record = self.record.unenrolled();
        self.persist().await?;
        let directory = Arc::clone(&self.directory);
        tokio::task::spawn_blocking(move || {
            directory.remove(netmap::CACHE_FILE)?;
            directory.remove(netmap::FACTS_FILE)?;
            netmap::artifacts::forget(&directory)
        })
        .await
        .map_err(|_| Error::ReloadRequired)??;
        Ok(())
    }
}
