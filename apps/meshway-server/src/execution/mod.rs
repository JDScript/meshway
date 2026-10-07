//! Authenticated node control and durable execution lifecycle.
pub(crate) mod connectivity;
pub(crate) mod diagnostics;
pub mod proof;
mod state;
use crate::error::Error;
use crate::{
    secrets::Sealer,
    store::{Handle, execution as store},
};
use meshway_model::*;
use meshway_protocol::{execution::*, node::*};
use rusqlite::TransactionBehavior;
use std::sync::Arc;

/// Created only by current credential validation. Never deserialize a principal.
#[derive(Clone)]
pub struct NodePrincipal {
    pub(crate) network: NetworkId,
    pub(crate) node: NodeId,
    pub(crate) key: KeyId,
    pub(crate) token_hash: [u8; 32],
    pub(crate) expires_at_ms: u64,
}
impl NodePrincipal {
    pub fn network_id(&self) -> NetworkId {
        self.network
    }
    pub fn node_id(&self) -> NodeId {
        self.node
    }
    pub fn key_id(&self) -> KeyId {
        self.key
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }
}

#[derive(Clone)]
pub struct Execution {
    handle: Handle,
    sealer: Arc<Sealer>,
    boot: InstanceId,
}
impl Execution {
    /// Current identity-scoped names, read and authorized in one transaction.
    pub async fn query_dns(
        &self,
        token: &str,
        request: NodeDnsQueryRequest,
        request_id: RequestId,
    ) -> Result<NodeDnsQueryResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .read(move |tx| store::query_dns(tx, &sealer, boot, token, request, request_id))
            .await
    }
    pub async fn revoke_key(
        &self,
        token: &str,
        network: NetworkId,
        node: NodeId,
        request: RevokeNodeKeyRequest,
    ) -> Result<(), Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let (_, effects) = store::revoke_key(tx, &sealer, token, network, node, request)?;
                Ok(((), effects))
            })
            .await
    }
    pub async fn select_profile(
        &self,
        token: &str,
        request: RoutingSelectionRequest,
    ) -> Result<RoutingSelectionResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::select_profile(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn settings(
        &self,
        token: &str,
        network: NetworkId,
        node: NodeId,
    ) -> Result<(Revision, meshway_configuration::NodeSettingsView), Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .read(move |tx| store::settings(tx, &sealer, token, network, node))
            .await
    }
    pub async fn inventory(
        &self,
        token: &str,
        network: NetworkId,
    ) -> Result<NodeInventoryResponse, Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .read(move |tx| store::inventory(tx, &sealer, token, network))
            .await
    }
    pub async fn progress(
        &self,
        token: &str,
        network: NetworkId,
        publication: PublicationId,
    ) -> Result<PublicationProgress, Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .read({
                let boot = self.boot;
                move |tx| store::progress(tx, &sealer, boot, token, network, publication)
            })
            .await
    }
    pub(crate) fn new(handle: Handle, sealer: Arc<Sealer>, boot: InstanceId) -> Self {
        Self {
            handle,
            sealer,
            boot,
        }
    }
    #[cfg(test)]
    pub(crate) fn signed_challenge_fixture(
        &self,
        request: NodeChallengeRequest,
        signer: &ed25519_dalek::SigningKey,
    ) -> meshway_protocol::SignedNodeChallengeRequest {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        use ed25519_dalek::Signer;
        let intent = meshway_protocol::NodeChallengeIntent {
            id: ChallengeId::new(crate::accounts::random_uuid().unwrap()).unwrap(),
            controller_id: self.sealer.installation_id(),
            request,
            expires_at_ms: store::now().unwrap() as u64 + 30_000,
        };
        let signature = signer.sign(&meshway_protocol::challenge_request_transcript(&intent));
        meshway_protocol::SignedNodeChallengeRequest {
            intent,
            signature: meshway_protocol::WireSecret::new(
                URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            ),
        }
    }
    pub async fn challenge(
        &self,
        request: meshway_protocol::SignedNodeChallengeRequest,
    ) -> Result<NodeChallenge, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .write(move |tx| store::challenge(tx, &sealer, request))
            .await
    }
    pub async fn session(&self, request: NodeSessionRequest) -> Result<NodeSessionResponse, Error> {
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .write(move |tx| store::session(tx, &sealer, boot, request))
            .await
    }
    pub async fn rotate(&self, request: NodeRotationRequest) -> Result<NodeKeyMetadata, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::rotate(tx, &sealer, request)
            })
            .await
    }
    pub async fn authenticate(&self, token: &str) -> Result<NodePrincipal, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .read(move |tx| store::authorize_node(tx, &sealer, boot, token, store::now()?))
            .await
    }
    pub async fn acquire(
        &self,
        token: &str,
        request: AcquireInstanceRequest,
    ) -> Result<InstanceLease, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::acquire(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn renew(
        &self,
        token: &str,
        request: InstanceRequest,
    ) -> Result<InstanceLease, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::renew(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn release(&self, token: &str, request: InstanceRequest) -> Result<(), Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::release(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn configuration(
        &self,
        token: &str,
        request: NodeConfigurationRequest,
    ) -> Result<NodeConfigurationResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::configuration(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn refresh_configuration(
        &self,
        token: &str,
        request: NodeConfigurationRequest,
    ) -> Result<NodeConfigurationResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::refresh_configuration(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn receipt(&self, token: &str, request: ReceiptRequest) -> Result<(), Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let result = store::receipt(tx, &sealer, boot, token, request.clone());
                if let Err(error) = &result {
                    tracing::info!(instance = %request.instance.instance_id,
                        generation = request.instance.generation.get(),
                        publication = %request.target.execution_publication,
                        attempt = request.attempt.get(), sequence = request.sequence.get(),
                        phase = ?request.phase, reason = %error, "Configuration receipt rejected");
                }
                result
            })
            .await
    }
    pub async fn demand(
        &self,
        token: &str,
        request: DemandRequest,
    ) -> Result<DemandResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::demand(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn renew_demand(
        &self,
        token: &str,
        request: RenewDemandRequest,
    ) -> Result<DemandResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::renew_demand(tx, &sealer, boot, token, request)
            })
            .await
    }
    pub async fn cancel_demand(
        &self,
        token: &str,
        request: CancelDemandRequest,
    ) -> Result<(), Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                store::cancel_demand(tx, &sealer, boot, token, request)
            })
            .await
    }
}
