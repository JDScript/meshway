//! Self-service deletion: the Server deletes this Node the way a manager
//! would (permanent revocation, netmap withdrawal), then the identity is
//! forgotten locally. Nothing local changes unless the Server confirms.
use crate::{Control, Error, netmap};
use meshway_model::RequestId;
use meshway_protocol::{NodeLeaveRequest, NodeLeaveResponse};
use std::sync::Arc;

impl Control {
    /// Requires a current session (see [`crate::shared::authenticate`]). A
    /// Server that cannot be reached or rejects the request leaves every local
    /// record untouched; only its confirmation clears them.
    pub async fn leave(&mut self, request_id: RequestId) -> Result<NodeLeaveResponse, Error> {
        self.ready()?;
        if self.record.rotation.is_some() || self.record.enrollment.is_some() {
            return Err(Error::Pending);
        }
        let binding = self.record.binding.clone().ok_or(Error::Lifecycle)?;
        let token = zeroize::Zeroizing::new(self.token()?.to_owned());
        let request = NodeLeaveRequest {
            request_id,
            node_id: binding.node_id,
        };
        let response = self.client.leave_node(&request, &token).await;
        let response = self.registered(response)?;
        if response.network_id != binding.network_id || response.node_id != binding.node_id {
            return Err(Error::Contract);
        }
        self.forget().await?;
        Ok(response)
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
            directory.remove(netmap::FACTS_FILE)
        })
        .await
        .map_err(|_| Error::ReloadRequired)??;
        Ok(())
    }
}
