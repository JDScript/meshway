//! Authenticated node control: sessions, instances, netmap receipts, State
//! admission and coordination. Execution authority itself is the netmap.
pub(crate) mod connectivity;
pub(crate) mod diagnostics;
pub mod proof;
mod state;
use crate::error::Error;
use crate::{
    secrets::Sealer,
    store::{Handle, execution as store},
};
pub use connectivity::netmap_udp_coordination;
pub use diagnostics::{NetmapDiagnostic, NetmapDiagnostics, netmap_diagnostic};
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
    netmaps: Arc<dyn crate::state::facts::NetmapReader>,
}
impl Execution {
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
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        boot: InstanceId,
        netmaps: Arc<dyn crate::state::facts::NetmapReader>,
    ) -> Self {
        Self {
            handle,
            sealer,
            boot,
            netmaps,
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
    /// The Node deletes itself through the one configuration commit: the
    /// same permanent revocation, netmap withdrawal and recompilation as a
    /// manager's deletion. Its credentials are checked in that transaction.
    pub(crate) async fn leave(
        &self,
        committer: crate::configuration::Committer,
        token: &str,
        request: NodeLeaveRequest,
    ) -> Result<NodeLeaveResponse, Error> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let principal = store::authorize_node(tx, &sealer, boot, token, store::now()?)?;
                if principal.node != request.node_id {
                    return Err(Error::PermissionDenied);
                }
                let effects = committer.commit(
                    tx,
                    principal.network,
                    crate::configuration::Intent::NodeLeave {
                        principal: &principal,
                    },
                    request.request_id,
                )?;
                let response = NodeLeaveResponse {
                    network_id: principal.network,
                    node_id: principal.node,
                    revision: effects.revision,
                };
                Ok((response, effects.changes()))
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
                        netmap_serial = request.netmap.serial.get(),
                        attempt = request.attempt.get(), sequence = request.sequence.get(),
                        phase = ?request.phase, reason = %error, "Configuration receipt rejected");
                }
                result
            })
            .await
    }
}
