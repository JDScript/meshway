//! Saved configuration pull/ACK; no cached authorization is activated here.
use crate::state::{
    Agent, AgentError,
    configuration::{self, SyncReceipt, cache_path},
    local,
};
use meshway_control_client::types::AcknowledgeConfigurationRequest;
use meshway_protocol::ConfigurationSnapshot;

impl Agent {
    pub async fn sync(&mut self) -> Result<SyncReceipt, AgentError> {
        self.prepare_control_plane().await?;
        local::ensure_durable(&self.path)?;
        if self.identity.node_id.is_none() {
            self.heartbeat().await?;
        }
        let client = self
            .controller_client()
            .with_api_key(&self.identity.node_credential);
        for _ in 0..3 {
            let wire = client.get_agent_configuration().await?;
            let snapshot: ConfigurationSnapshot = serde_json::from_value(
                serde_json::to_value(wire)
                    .map_err(|_| AgentError::Invalid("cannot decode configuration"))?,
            )
            .map_err(|_| AgentError::Invalid("invalid configuration response"))?;
            #[cfg(feature = "managed-connections")]
            if let Some(guard) = &self.dns_guard {
                guard.observe_revision(snapshot.payload.revision);
            }
            configuration::validate(&snapshot, &self.identity, self.cache.as_ref())?;
            self.persist_configuration(snapshot)?;
            // ACK the durable visible value even when a previous final fsync failed.
            local::ensure_durable(&cache_path(&self.path))?;
            let cache = self.cache.as_ref().expect("persisted cache");
            let response = client
                .acknowledge_configuration(AcknowledgeConfigurationRequest {
                    revision: cache.snapshot.payload.revision.into(),
                    digest: cache.snapshot.digest.clone(),
                })
                .await;
            match response {
                Err(error) if error.api().is_some_and(|error| error.status == 409) => {
                    #[cfg(feature = "managed-connections")]
                    if let Some(guard) = &self.dns_guard {
                        guard.conflict(cache.snapshot.payload.revision);
                    }
                    continue;
                }
                Err(error) => return Err(error.into()),
                Ok(receipt) => {
                    if receipt.revision != i64::from(cache.snapshot.payload.revision)
                        || receipt.digest != cache.snapshot.digest
                    {
                        return Err(AgentError::Invalid(
                            "ACK receipt does not match saved configuration",
                        ));
                    }
                    return Ok(SyncReceipt {
                        controller: self.identity.controller.clone(),
                        node_id: cache.node_id.clone(),
                        stored_revision: cache.snapshot.payload.revision,
                        digest: cache.snapshot.digest.clone(),
                        mesh_ipv4: cache.snapshot.payload.node.mesh_ipv4.clone(),
                        mesh_ipv6: cache.snapshot.payload.node.mesh_ipv6.clone(),
                    });
                }
            }
        }
        Err(AgentError::Http {
            status: 409,
            retryable: true,
        })
    }
}
