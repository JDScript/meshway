//! One-use admission requests and keys. The configuration commit runs inside
//! the enrollment transaction through the configuration owner, never as a
//! second commit.
use crate::error::Error;
pub(crate) mod settings;
use crate::{
    secrets::Sealer,
    store::{Handle, enrollment as store},
};
use meshway_model::*;
use meshway_protocol::enrollment::*;
use rusqlite::TransactionBehavior;
use std::sync::Arc;
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
    commit: crate::configuration::Committer,
}
impl Enrollment {
    pub async fn redeem_key(
        &self,
        request: KeyEnrollmentRequest,
    ) -> Result<EnrollmentResponse, Error> {
        let commit = self.commit.clone();
        let id = request.request.intent.id;
        self.create_request(request.request).await?;
        let sealer = self.sealer.clone();
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let (result, effects) = store::redeem_key_in_transaction(
                    tx,
                    &sealer,
                    id,
                    request.enrollment_key,
                    request.verifier,
                    &commit,
                )?;
                Ok((result, effects.map(|e| e.changes()).unwrap_or_default()))
            })
            .await
    }
    pub async fn redeem(
        &self,
        id: EnrollmentRequestId,
        request: RedeemEnrollmentRequest,
    ) -> Result<EnrollmentResponse, Error> {
        let commit = self.commit.clone();
        let sealer = self.sealer.clone();
        self.handle
            .transact(TransactionBehavior::Immediate, move |tx| {
                let (result, effects) =
                    store::redeem_in_transaction(tx, &sealer, id, request, &commit)?;
                Ok((result, effects.map(|e| e.changes()).unwrap_or_default()))
            })
            .await
    }
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        commit: crate::configuration::Committer,
    ) -> Self {
        Self {
            handle,
            sealer,
            commit,
        }
    }
    pub async fn create_request(
        &self,
        request: CreateEnrollmentRequest,
    ) -> Result<EnrollmentRequestView, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .write(move |tx| store::create_request(tx, &sealer, request))
            .await
    }
    pub async fn inspect(
        &self,
        token: &str,
        id: EnrollmentRequestId,
    ) -> Result<DeviceAuthorizationView, Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .read(move |tx| store::inspect(tx, token, id))
            .await
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
            .write(move |tx| store::approve(tx, &sealer, token, id, request))
            .await
    }
    pub async fn deny(&self, token: &str, id: EnrollmentRequestId) -> Result<(), Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .write(move |tx| store::deny(tx, token, id))
            .await
    }
    pub async fn poll(
        &self,
        id: EnrollmentRequestId,
        proof: EnrollmentDeviceProof,
    ) -> Result<EnrollmentPollResponse, Error> {
        let sealer = self.sealer.clone();
        self.handle
            .read(move |tx| store::poll(tx, &sealer, id, proof))
            .await
    }
    pub async fn cancel(
        &self,
        id: EnrollmentRequestId,
        proof: EnrollmentDeviceProof,
    ) -> Result<(), Error> {
        self.handle
            .write(move |tx| store::cancel(tx, id, proof))
            .await
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
            .write(move |tx| store::create_key(tx, &sealer, token, network, request))
            .await
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
            .read(move |tx| store::list_keys(tx, token, network, limit, cursor, &sealer))
            .await
    }
    pub async fn revoke_key(
        &self,
        token: &str,
        network: NetworkId,
        key: EnrollmentKeyId,
    ) -> Result<(), Error> {
        let token = crate::accounts::token_digest(token)?;
        self.handle
            .write(move |tx| store::revoke_key(tx, token, network, key))
            .await
    }
}
