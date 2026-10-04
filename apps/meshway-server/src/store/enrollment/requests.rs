use super::{hash, pkce, redeem::validate_existing, token};
use crate::error::Error;
use crate::execution::proof;
use crate::secrets::Sealer;
use crate::store::{
    authorization::current_session,
    execution::{bytes, id, json, now},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use meshway_model::{
    Digest, EnrollmentRequestId, KeyAlgorithm, MembershipId, NetworkId, NodeId, NodePublicKey,
    UserId,
};
use meshway_protocol::{
    WireSecret,
    enrollment::{
        ApproveEnrollmentRequest, CreateEnrollmentRequest, DeviceAuthorizationView,
        EnrollmentApprovalView, EnrollmentDeviceProof, EnrollmentPhase, EnrollmentPollResponse,
        EnrollmentRequestView, EnrollmentReturnMethod, ExistingNodeBinding, enrollment_transcript,
    },
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub(super) struct Approval {
    pub network: NetworkId,
    pub user: UserId,
    pub membership: MembershipId,
    pub session_hash: Digest,
}
pub(super) struct Pending {
    pub id: EnrollmentRequestId,
    pub control: [u8; 32],
    pub dataplane: [u8; 32],
    pub digest: Digest,
    pub challenge: String,
    pub hostname: String,
    pub expires: i64,
    pub approval: Option<Approval>,
    pub result_hash: Option<Vec<u8>>,
    pub delivery: Option<Vec<u8>>,
    pub denied: Option<i64>,
    pub cancelled: Option<i64>,
    pub consumed: Option<i64>,
    pub node: Option<NodeId>,
    pub existing: Option<ExistingNodeBinding>,
}
impl Pending {
    pub(super) fn view(&self, now: i64) -> EnrollmentRequestView {
        EnrollmentRequestView {
            id: self.id,
            hostname: self.hostname.clone(),
            existing: self.existing.clone(),
            return_method: EnrollmentReturnMethod::Poll,
            control_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: self.control.into(),
            },
            phase: if self.consumed.is_some() {
                EnrollmentPhase::Consumed
            } else if self.cancelled.is_some() {
                EnrollmentPhase::Cancelled
            } else if self.denied.is_some() {
                EnrollmentPhase::Withdrawn
            } else if self.expires <= now {
                EnrollmentPhase::Expired
            } else if self.approval.is_some() {
                EnrollmentPhase::Approved
            } else {
                EnrollmentPhase::Pending
            },
            expires_at_ms: self.expires as u64,
        }
    }
    pub(super) fn device(
        &self,
        proof: &EnrollmentDeviceProof,
        operation: &str,
    ) -> Result<(), Error> {
        pkce(&self.challenge, proof.verifier.expose())?;
        proof::verify_bytes(
            &self.control,
            &enrollment_transcript(self.id, self.digest, operation),
            proof.signature.expose(),
        )
    }
}
pub(super) fn pending(
    tx: &Transaction<'_>,
    request: EnrollmentRequestId,
) -> Result<Pending, Error> {
    let raw = tx.query_row(
        "SELECT control_key,dataplane_key,request_digest,code_challenge,hostname,expires_at,
                approval,result_hash,delivery,denied_at,consumed_at,node_id,existing_binding,cancelled_at
         FROM enrollment_requests WHERE id=?1",
        [request.as_uuid().as_bytes().as_slice()],
        |r| Ok((
            r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?, r.get::<_, Vec<u8>>(2)?,
            r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?,
            r.get::<_, Option<Vec<u8>>>(6)?, r.get::<_, Option<Vec<u8>>>(7)?, r.get::<_, Option<Vec<u8>>>(8)?,
            r.get::<_, Option<i64>>(9)?, r.get::<_, Option<i64>>(10)?,
            r.get::<_, Option<Vec<u8>>>(11)?, r.get::<_, Option<Vec<u8>>>(12)?, r.get::<_, Option<i64>>(13)?,
        )),
    ).optional()?.ok_or(Error::NotFound)?;
    Ok(Pending {
        id: request,
        control: raw
            .0
            .try_into()
            .map_err(|_| Error::decode("enrollment_requests.control_key"))?,
        dataplane: raw
            .1
            .try_into()
            .map_err(|_| Error::decode("enrollment_requests.dataplane_key"))?,
        digest: Digest::from_bytes(
            raw.2
                .try_into()
                .map_err(|_| Error::decode("enrollment_requests.request_digest"))?,
        ),
        challenge: raw.3,
        hostname: raw.4,
        expires: raw.5,
        approval: raw.6.map(|b| json(&b)).transpose()?,
        result_hash: raw.7,
        delivery: raw.8,
        denied: raw.9,
        cancelled: raw.13,
        consumed: raw.10,
        node: raw.11.map(id).transpose()?,
        existing: raw.12.map(|b| json(&b)).transpose()?,
    })
}
pub(crate) fn create_request(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    request: CreateEnrollmentRequest,
) -> Result<EnrollmentRequestView, Error> {
    let i = request.intent;
    if i.control_key.algorithm != KeyAlgorithm::Ed25519
        || i.dataplane_key.algorithm != KeyAlgorithm::X25519
        || i.control_key.bytes == i.dataplane_key.bytes
        || i.dataplane_key.bytes.as_bytes() == &[0; 32]
        || !meshway_configuration::valid_node_hostname(&i.hostname)
        || i.code_challenge.len() != 43
        || URL_SAFE_NO_PAD.decode(&i.code_challenge).is_err()
    {
        return Err(Error::InvalidRequest);
    }
    let digest = proof::request_digest(&i)?;
    proof::verify_bytes(
        i.control_key.bytes.as_bytes(),
        &enrollment_transcript(i.id, digest, "create"),
        request.signature.expose(),
    )?;
    let now = now()?;
    if let Some(existing) = &i.existing {
        validate_existing(
            tx,
            sealer,
            existing,
            i.control_key.bytes.as_bytes(),
            i.dataplane_key.bytes.as_bytes(),
        )?;
    }
    if let Ok(p) = pending(tx, i.id) {
        if p.digest != digest {
            return Err(Error::BindingConflict);
        }
        return Ok(p.view(now));
    }
    let count:i64=tx.query_row("SELECT count(*) FROM enrollment_requests WHERE expires_at>?1 AND consumed_at IS NULL AND denied_at IS NULL AND cancelled_at IS NULL",[now],|r|r.get(0))?;
    if count >= 4096 {
        return Err(Error::Busy);
    }
    tx.execute("INSERT INTO enrollment_requests(id,control_key,dataplane_key,request_digest,code_challenge,hostname,created_at,expires_at,existing_binding) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![i.id.as_uuid().as_bytes().as_slice(),i.control_key.bytes.as_bytes().as_slice(),i.dataplane_key.bytes.as_bytes().as_slice(),digest.as_bytes().as_slice(),i.code_challenge,i.hostname,now,now+600_000,i.existing.as_ref().map(bytes).transpose()?])?;
    let view = pending(tx, i.id)?.view(now);
    Ok(view)
}
pub(crate) fn inspect(
    tx: &Transaction<'_>,
    token: [u8; 32],
    id: EnrollmentRequestId,
) -> Result<DeviceAuthorizationView, Error> {
    let now = now()?;
    let user = current_session(tx, &token, now)?.user.id;
    let p = pending(tx, id)?;
    user_view(tx, &p, user, now)
}
fn user_view(
    tx: &Transaction<'_>,
    p: &Pending,
    user: UserId,
    now: i64,
) -> Result<DeviceAuthorizationView, Error> {
    let approval = p
        .approval
        .as_ref()
        .filter(|a| {
            a.user == user
                || crate::store::authorization::authorize_manager(tx, a.network, user).is_ok()
        })
        .map(|a| EnrollmentApprovalView {
            user_id: a.user,
            network_id: a.network,
        });
    Ok(DeviceAuthorizationView {
        request: p.view(now),
        approval,
    })
}
pub(crate) fn approve(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token_hash: [u8; 32],
    id: EnrollmentRequestId,
    request: ApproveEnrollmentRequest,
) -> Result<DeviceAuthorizationView, Error> {
    let now = now()?;
    let session = current_session(tx, &token_hash, now)?;
    let membership:Vec<u8>=tx.query_row("SELECT m.id FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",params![request.network_id.as_uuid().as_bytes().as_slice(),session.user.id.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(Error::PermissionDenied)?;
    let p = pending(tx, id)?;
    if p.existing
        .as_ref()
        .is_some_and(|e| e.network_id != request.network_id)
    {
        return Err(Error::PermissionDenied);
    }
    if p.expires <= now || p.denied.is_some() || p.cancelled.is_some() || p.consumed.is_some() {
        return Err(Error::StateConflict);
    }
    if let Some(a) = &p.approval {
        if a.network != request.network_id || a.user != session.user.id {
            return Err(Error::StateConflict);
        }
        return user_view(tx, &p, session.user.id, now);
    }
    let approval = Approval {
        network: request.network_id,
        user: session.user.id,
        membership: super::id(membership)?,
        session_hash: token_hash.into(),
    };
    let code = token()?;
    let delivery = sealer.seal_enrollment(id.into_uuid(), code.as_bytes())?;
    tx.execute(
        "UPDATE enrollment_requests SET approval=?1,result_hash=?2,delivery=?3 WHERE id=?4",
        params![
            bytes(&approval)?,
            hash(&code).as_slice(),
            delivery.as_bytes(),
            id.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let view = user_view(tx, &pending(tx, id)?, session.user.id, now)?;
    Ok(view)
}
pub(crate) fn deny(
    tx: &Transaction<'_>,
    token: [u8; 32],
    id: EnrollmentRequestId,
) -> Result<(), Error> {
    let now = now()?;
    let session = current_session(tx, &token, now)?;
    let p = pending(tx, id)?;
    // Before approval there is no bound user. Leaving the browser is local;
    // only the device's signed cancel operation can cancel a pending request.
    let approval = p.approval.ok_or(Error::PermissionDenied)?;
    if approval.user != session.user.id {
        return Err(Error::PermissionDenied);
    }
    if p.consumed.is_some() || p.cancelled.is_some() || p.expires <= now {
        return Err(Error::StateConflict);
    }
    tx.execute(
        "UPDATE enrollment_requests SET denied_at=COALESCE(denied_at,?1),delivery=NULL WHERE id=?2",
        params![now, id.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}
pub(crate) fn poll(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    id: EnrollmentRequestId,
    proof: EnrollmentDeviceProof,
) -> Result<EnrollmentPollResponse, Error> {
    let now = now()?;
    let p = pending(tx, id)?;
    p.device(&proof, "poll")?;
    let code =
        if p.expires > now && p.denied.is_none() && p.cancelled.is_none() && p.consumed.is_none() {
            p.delivery
                .as_ref()
                .map(|d| {
                    let plaintext = sealer.open_enrollment(id.into_uuid(), d)?;
                    Ok::<_, Error>(WireSecret::new(
                        std::str::from_utf8(plaintext.as_bytes())
                            .map_err(|_| Error::decode("enrollment_requests.delivery"))?
                            .to_owned(),
                    ))
                })
                .transpose()?
        } else {
            None
        };
    Ok(EnrollmentPollResponse {
        approval: p.approval.as_ref().map(|a| EnrollmentApprovalView {
            user_id: a.user,
            network_id: a.network,
        }),
        request: p.view(now),
        code,
        node_id: p.node,
    })
}
pub(crate) fn cancel(
    tx: &Transaction<'_>,
    id: EnrollmentRequestId,
    proof: EnrollmentDeviceProof,
) -> Result<(), Error> {
    let p = pending(tx, id)?;
    p.device(&proof, "cancel")?;
    let now = now()?;
    if p.consumed.is_some() || p.denied.is_some() || p.expires <= now {
        return Err(Error::StateConflict);
    }
    tx.execute(
        "UPDATE enrollment_requests SET cancelled_at=COALESCE(cancelled_at,?1),delivery=NULL WHERE id=?2",
        params![now, id.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(())
}
