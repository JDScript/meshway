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
    /// Supplies the State report/view provider with actual current SQL facts. The
    /// returned configuration is private execution material: providers project
    /// only resource identities/allowed endpoints and never broadcast it.
    pub fn current_node_execution(
        &self,
        tx: &rusqlite::Transaction<'_>,
        token: &str,
    ) -> Result<
        (
            NodePrincipal,
            InstanceLease,
            Option<meshway_configuration::execution::CompiledNode>,
        ),
        Error,
    > {
        let now = store::now()?;
        let p = store::authorize_node(tx, &self.sealer, self.boot, proof::token_hash(token)?, now)?;
        let lease = store::instance_lease(tx, &self.sealer, &p)?;
        store::current_instance(
            tx,
            self.boot,
            &p,
            &InstanceRequest {
                instance_id: lease.instance_id,
                generation: lease.generation,
            },
            now,
        )?;
        let target = store::current_target(tx, &self.sealer, &p)?.map(|(_, target)| target);
        Ok((p, lease, target))
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
    /// Atomic seam for State/observation storage. Every use rechecks the current
    /// session, Config identity, instance and (when requested) execution lease.
    pub fn authorize_in_transaction(
        &self,
        tx: &rusqlite::Transaction<'_>,
        token: &str,
        instance: &InstanceRequest,
        configuration: Option<&ConfigurationRef>,
        require_execution: bool,
    ) -> Result<NodePrincipal, Error> {
        let now = store::now()?;
        let p = store::authorize_node(tx, &self.sealer, self.boot, proof::token_hash(token)?, now)?;
        store::current_instance(tx, self.boot, &p, instance, now)?;
        let lease = store::instance_lease(tx, &self.sealer, &p)?;
        if configuration.is_some_and(|c| lease.target.as_ref() != Some(c)) {
            return Err(Error::BindingConflict);
        }
        if require_execution {
            if lease
                .execution_expires_at_ms
                .is_none_or(|t| t <= now as u64)
            {
                return Err(Error::BindingConflict);
            }
            let target = lease.target.ok_or(Error::BindingConflict)?;
            store::check_publication(
                tx,
                &self.sealer,
                p.network,
                p.node,
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
    /// State/observation owner obtains current identity and instance admission;
    /// callers requiring atomic report writes use the same store functions in
    /// their transaction rather than trusting this past check as a capability.
    pub async fn validate_instance(
        &self,
        principal: NodePrincipal,
        request: InstanceRequest,
    ) -> Result<(), Error> {
        let sealer = self.sealer.clone();
        let boot = self.boot;
        self.handle
            .read(move |tx| {
                let now = store::now()?;
                let p = store::reauthorize(tx, &sealer, boot, &principal, now)?;
                store::current_instance(tx, boot, &p, &request, now)
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
    pub async fn challenge(&self, request: NodeChallengeRequest) -> Result<NodeChallenge, Error> {
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
                store::receipt(tx, &sealer, boot, token, request)
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
