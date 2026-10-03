//! Authenticated node control and durable execution lifecycle.
pub(crate) mod configuration;
pub mod proof;
use crate::{
    secrets::Sealer,
    store::{Handle, execution as store},
};
use meshway_model::*;
use meshway_protocol::{execution::*, node::*};
use std::sync::Arc;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NodeError {
    #[error("invalid node credentials or proof")]
    Unauthorized,
    #[error("node operation is not permitted")]
    Forbidden,
    #[error("invalid node request")]
    Invalid,
    #[error("node request conflicts with current generation or binding")]
    Conflict,
    #[error("node resource not found")]
    NotFound,
    #[error("node operation budget exhausted")]
    Busy,
    #[error("node storage operation failed")]
    Storage,
}
impl From<rusqlite::Error> for NodeError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
impl From<crate::store::ExecutorError> for NodeError {
    fn from(_: crate::store::ExecutorError) -> Self {
        Self::Storage
    }
}
impl From<crate::accounts::AccountError> for NodeError {
    fn from(_: crate::accounts::AccountError) -> Self {
        Self::Unauthorized
    }
}
impl From<meshway_configuration::compile::CompileError> for NodeError {
    fn from(e: meshway_configuration::compile::CompileError) -> Self {
        match e {
            meshway_configuration::compile::CompileError::Budget => Self::Busy,
            _ => Self::Invalid,
        }
    }
}

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
    pub async fn revoke_key(
        &self,
        token: &str,
        network: NetworkId,
        node: NodeId,
        request: RevokeNodeKeyRequest,
    ) -> Result<(), NodeError> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .execute(move |c| store::revoke_key(c, token, network, node, request))
            .await?
    }
    /// Atomic seam for State/observation storage. Every use rechecks the current
    /// session, Config identity, instance and (when requested) execution lease.
    pub fn authorize_in_transaction(
        &self,
        tx: &rusqlite::Transaction<'_>,
        token: &str,
        instance: &InstanceRequest,
        configuration: Option<&ConfigurationRef>,
        require_execution: bool,
    ) -> Result<NodePrincipal, NodeError> {
        let now = store::now()?;
        let p = store::authorize_node(tx, &self.sealer, self.boot, proof::token_hash(token)?, now)?;
        store::current_instance(tx, self.boot, &p, instance, now)?;
        let lease = store::instance_lease(tx, &self.sealer, &p)?;
        if configuration.is_some_and(|c| lease.target.as_ref() != Some(c)) {
            return Err(NodeError::Conflict);
        }
        if require_execution {
            if lease
                .execution_expires_at_ms
                .is_none_or(|t| t <= now as u64)
            {
                return Err(NodeError::Conflict);
            }
            let target = lease.target.ok_or(NodeError::Conflict)?;
            store::check_publication(
                tx,
                &self.sealer,
                p.network,
                target.execution_publication,
                now,
            )?;
        }
        Ok(p)
    }
    pub async fn select_profile(
        &self,
        token: &str,
        request: RoutingSelectionRequest,
    ) -> Result<RoutingSelectionResponse, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::select_profile(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn inventory(
        &self,
        token: &str,
        network: NetworkId,
    ) -> Result<NodeInventoryResponse, NodeError> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::inventory(c, &sealer, token, network))
            .await?
    }
    pub async fn progress(
        &self,
        token: &str,
        network: NetworkId,
        publication: PublicationId,
    ) -> Result<PublicationProgress, NodeError> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::progress(c, &sealer, token, network, publication))
            .await?
    }
    /// Config owner invokes this before its SQL commit, after the authoritative
    /// complete version/current pointer have been written. No second transaction.
    pub fn apply_configuration_in_transaction(
        &self,
        tx: &rusqlite::Transaction<'_>,
        network: NetworkId,
        previous: &meshway_configuration::NetworkConfiguration,
        target: &meshway_configuration::NetworkConfiguration,
    ) -> Result<PublicationId, NodeError> {
        store::apply_configuration_effects(
            tx,
            &self.sealer,
            network,
            previous,
            target,
            store::now()?,
        )
    }
    /// State/observation owner obtains current identity and instance admission;
    /// callers requiring atomic report writes use the same store functions in
    /// their transaction rather than trusting this past check as a capability.
    pub async fn validate_instance(
        &self,
        principal: NodePrincipal,
        request: InstanceRequest,
    ) -> Result<(), NodeError> {
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| {
                let tx = c.transaction()?;
                let now = store::now()?;
                let p = store::reauthorize(&tx, &sealer, boot, &principal, now)?;
                store::current_instance(&tx, boot, &p, &request, now)
            })
            .await?
    }
    pub(crate) fn new(handle: Handle, sealer: Arc<Sealer>, boot: InstanceId) -> Self {
        Self {
            handle,
            sealer,
            boot,
        }
    }
    pub async fn challenge(
        &self,
        request: NodeChallengeRequest,
    ) -> Result<NodeChallenge, NodeError> {
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::challenge(c, &sealer, request))
            .await?
    }
    pub async fn session(
        &self,
        request: NodeSessionRequest,
    ) -> Result<NodeSessionResponse, NodeError> {
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::session(c, &sealer, boot, request))
            .await?
    }
    pub async fn rotate(&self, request: NodeRotationRequest) -> Result<NodeKeyMetadata, NodeError> {
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::rotate(c, &sealer, request))
            .await?
    }
    pub async fn authenticate(&self, token: &str) -> Result<NodePrincipal, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| {
                let tx = c.transaction()?;
                store::authorize_node(&tx, &sealer, boot, token, store::now()?)
            })
            .await?
    }
    pub async fn acquire(
        &self,
        token: &str,
        request: AcquireInstanceRequest,
    ) -> Result<InstanceLease, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::acquire(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn renew(
        &self,
        token: &str,
        request: InstanceRequest,
    ) -> Result<InstanceLease, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::renew(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn release(&self, token: &str, request: InstanceRequest) -> Result<(), NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::release(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn configuration(
        &self,
        token: &str,
        request: ConfigurationRequest,
    ) -> Result<ConfigurationResponse, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::configuration(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn receipt(&self, token: &str, request: ReceiptRequest) -> Result<(), NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::receipt(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn demand(
        &self,
        token: &str,
        request: DemandRequest,
    ) -> Result<DemandResponse, NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::demand(c, &sealer, boot, token, request))
            .await?
    }
    pub async fn cancel_demand(
        &self,
        token: &str,
        request: CancelDemandRequest,
    ) -> Result<(), NodeError> {
        let token = proof::token_hash(token)?;
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .execute(move |c| store::cancel_demand(c, &sealer, boot, token, request))
            .await?
    }
}
