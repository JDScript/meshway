//! One-use admission requests and keys. Config commit is supplied by its owner
//! and executes inside the enrollment transaction, never as a second commit.
use crate::error::Error;
pub(crate) mod configuration;
pub(crate) mod settings;
use crate::{
    secrets::Sealer,
    store::{Handle, enrollment as store},
};
use meshway_model::*;
use meshway_protocol::enrollment::*;
use std::sync::Arc;
/// Transaction participant supplied by the configuration owner. It normalizes
/// and persists the complete source target, its stable bindings and publication.
pub(crate) trait EnrollmentCommit: Send + Sync {
    fn insert_node(
        &self,
        tx: &rusqlite::Transaction<'_>,
        admission: &NodeAdmission,
    ) -> Result<crate::configuration::CommitEffects, Error>;
}
pub struct NodeAdmission {
    pub(crate) proof: crate::store::enrollment::VerifiedEnrollmentProof,
    pub network: NetworkId,
    pub node: NodeId,
    pub request: EnrollmentRequestId,
    pub identity: meshway_configuration::NodeIdentity,
    pub membership: Option<MembershipId>,
    pub actor: UserId,
    pub initial: InitialNodeSettings,
}
#[derive(Clone)]
pub struct Enrollment {
    handle: Handle,
    sealer: Arc<Sealer>,
    commit: Arc<dyn EnrollmentCommit>,
    publisher: crate::state::service::CommitPublisher,
}
impl Enrollment {
    pub async fn redeem_key(
        &self,
        request: KeyEnrollmentRequest,
    ) -> Result<EnrollmentResponse, Error> {
        let commit = self.commit.clone();
        let publisher = self.publisher.clone();
        let id = request.request.intent.id;
        self.create_request(request.request).await?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let (result, effects) = store::redeem_key_in_transaction(
                    &tx,
                    &sealer,
                    id,
                    request.enrollment_key,
                    request.verifier,
                    commit.as_ref(),
                )?;
                tx.commit()?;
                for change in effects.map(|e| e.changes()).unwrap_or_default() {
                    publisher.publish_committed(change);
                }
                Ok(result)
            })
            .await?
    }
    pub async fn redeem(
        &self,
        id: EnrollmentRequestId,
        request: RedeemEnrollmentRequest,
    ) -> Result<EnrollmentResponse, Error> {
        let commit = self.commit.clone();
        let publisher = self.publisher.clone();
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let (result, effects) =
                    store::redeem_in_transaction(&tx, &sealer, id, request, commit.as_ref())?;
                tx.commit()?;
                for change in effects.map(|e| e.changes()).unwrap_or_default() {
                    publisher.publish_committed(change);
                }
                Ok(result)
            })
            .await?
    }
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        commit: Arc<dyn EnrollmentCommit>,
        publisher: crate::state::service::CommitPublisher,
    ) -> Self {
        Self {
            handle,
            sealer,
            commit,
            publisher,
        }
    }
    pub async fn create_request(
        &self,
        request: CreateEnrollmentRequest,
    ) -> Result<EnrollmentRequestView, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::create_request(c, &sealer, request))
            .await?
    }
    pub async fn inspect(
        &self,
        token: &str,
        id: EnrollmentRequestId,
    ) -> Result<DeviceAuthorizationView, Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .execute(move |c| store::inspect(c, token, id))
            .await?
    }
    pub async fn approve(
        &self,
        token: &str,
        id: EnrollmentRequestId,
        request: ApproveEnrollmentRequest,
    ) -> Result<DeviceAuthorizationView, Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::approve(c, &sealer, token, id, request))
            .await?
    }
    pub async fn deny(&self, token: &str, id: EnrollmentRequestId) -> Result<(), Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .execute(move |c| store::deny(c, token, id))
            .await?
    }
    pub async fn poll(
        &self,
        id: EnrollmentRequestId,
        proof: EnrollmentDeviceProof,
    ) -> Result<EnrollmentPollResponse, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::poll(c, &sealer, id, proof))
            .await?
    }
    pub async fn cancel(
        &self,
        id: EnrollmentRequestId,
        proof: EnrollmentDeviceProof,
    ) -> Result<(), Error> {
        self.handle
            .execute(move |c| store::cancel(c, id, proof))
            .await?
    }
    pub async fn create_key(
        &self,
        token: &str,
        network: NetworkId,
        request: CreateEnrollmentKeyRequest,
    ) -> Result<EnrollmentKeyResponse, Error> {
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::create_key(c, &sealer, token, network, request))
            .await?
    }
    pub async fn list_keys(
        &self,
        token: &str,
        network: NetworkId,
        limit: u16,
        cursor: Option<String>,
    ) -> Result<EnrollmentKeysResponse, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidRequest);
        }
        let token = crate::accounts::token_digest(token)?;
        let sealer = self.sealer.clone();
        self.handle
            .execute(move |c| store::list_keys(c, token, network, limit, cursor, &sealer))
            .await?
    }
    pub async fn revoke_key(
        &self,
        token: &str,
        network: NetworkId,
        key: EnrollmentKeyId,
    ) -> Result<(), Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .execute(move |c| store::revoke_key(c, token, network, key))
            .await?
    }
}
