use super::*;
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
    pub consumed: Option<i64>,
    pub node: Option<NodeId>,
}
impl Pending {
    pub(super) fn view(&self, now: i64) -> EnrollmentRequestView {
        EnrollmentRequestView {
            id: self.id,
            hostname: self.hostname.clone(),
            control_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: self.control.into(),
            },
            phase: if self.consumed.is_some() {
                EnrollmentPhase::Consumed
            } else if self.denied.is_some() {
                EnrollmentPhase::Denied
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
    ) -> Result<(), NodeError> {
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
) -> Result<Pending, NodeError> {
    let raw=tx.query_row("SELECT control_key,dataplane_key,request_digest,code_challenge,hostname,expires_at,approval,result_hash,delivery,denied_at,consumed_at,node_id FROM enrollment_requests WHERE id=?1",[request.as_uuid().as_bytes().as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?,r.get::<_,Option<Vec<u8>>>(6)?,r.get::<_,Option<Vec<u8>>>(7)?,r.get::<_,Option<Vec<u8>>>(8)?,r.get::<_,Option<i64>>(9)?,r.get::<_,Option<i64>>(10)?,r.get::<_,Option<Vec<u8>>>(11)?))).optional()?.ok_or(NodeError::NotFound)?;
    Ok(Pending {
        id: request,
        control: raw.0.try_into().map_err(|_| NodeError::Storage)?,
        dataplane: raw.1.try_into().map_err(|_| NodeError::Storage)?,
        digest: Digest::from_bytes(raw.2.try_into().map_err(|_| NodeError::Storage)?),
        challenge: raw.3,
        hostname: raw.4,
        expires: raw.5,
        approval: raw.6.map(|b| json(&b)).transpose()?,
        result_hash: raw.7,
        delivery: raw.8,
        denied: raw.9,
        consumed: raw.10,
        node: raw.11.map(id).transpose()?,
    })
}
pub(crate) fn create_request(
    c: &mut Connection,
    _sealer: &Sealer,
    request: CreateEnrollmentRequest,
) -> Result<EnrollmentRequestView, NodeError> {
    let i = request.intent;
    if i.control_key.algorithm != KeyAlgorithm::Ed25519
        || i.dataplane_key.algorithm != KeyAlgorithm::X25519
        || i.control_key.bytes == i.dataplane_key.bytes
        || i.dataplane_key.bytes.as_bytes() == &[0; 32]
        || i.hostname.is_empty()
        || i.hostname.len() > 63
        || i.hostname.starts_with('-')
        || i.hostname.ends_with('-')
        || !i
            .hostname
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || i.code_challenge.len() != 43
        || URL_SAFE_NO_PAD.decode(&i.code_challenge).is_err()
    {
        return Err(NodeError::Invalid);
    }
    let digest = proof::request_digest(&i)?;
    proof::verify_bytes(
        i.control_key.bytes.as_bytes(),
        &enrollment_transcript(i.id, digest, "create"),
        request.signature.expose(),
    )?;
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    if let Ok(p) = pending(&tx, i.id) {
        if p.digest != digest {
            return Err(NodeError::Conflict);
        }
        return Ok(p.view(now));
    }
    let count:i64=tx.query_row("SELECT count(*) FROM enrollment_requests WHERE expires_at>?1 AND consumed_at IS NULL AND denied_at IS NULL",[now],|r|r.get(0))?;
    if count >= 4096 {
        return Err(NodeError::Busy);
    }
    tx.execute("INSERT INTO enrollment_requests(id,control_key,dataplane_key,request_digest,code_challenge,hostname,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![i.id.as_uuid().as_bytes().as_slice(),i.control_key.bytes.as_bytes().as_slice(),i.dataplane_key.bytes.as_bytes().as_slice(),digest.as_bytes().as_slice(),i.code_challenge,i.hostname,now,now+600_000])?;
    let view = pending(&tx, i.id)?.view(now);
    tx.commit()?;
    Ok(view)
}
pub(crate) fn inspect(
    c: &mut Connection,
    token: [u8; 32],
    id: EnrollmentRequestId,
) -> Result<EnrollmentRequestView, NodeError> {
    let tx = c.transaction()?;
    let now = now()?;
    current_session(&tx, &token, now)?;
    Ok(pending(&tx, id)?.view(now))
}
pub(crate) fn approve(
    c: &mut Connection,
    sealer: &Sealer,
    token_hash: [u8; 32],
    id: EnrollmentRequestId,
    request: ApproveEnrollmentRequest,
) -> Result<EnrollmentRequestView, NodeError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let session = current_session(&tx, &token_hash, now)?;
    let membership:Vec<u8>=tx.query_row("SELECT m.id FROM memberships m JOIN networks n ON n.id=m.network_id WHERE m.network_id=?1 AND m.user_id=?2 AND m.revoked_at IS NULL AND n.deleted_at IS NULL",params![request.network_id.as_uuid().as_bytes().as_slice(),session.user.id.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(NodeError::Forbidden)?;
    let p = pending(&tx, id)?;
    if p.expires <= now || p.denied.is_some() || p.consumed.is_some() {
        return Err(NodeError::Conflict);
    }
    if let Some(a) = p.approval {
        if a.network != request.network_id || a.user != session.user.id {
            return Err(NodeError::Conflict);
        }
        return Ok(pending(&tx, id)?.view(now));
    }
    let approval = Approval {
        network: request.network_id,
        user: session.user.id,
        membership: super::id(membership)?,
        session_hash: token_hash.into(),
    };
    let code = token()?;
    let delivery = sealer
        .seal_enrollment(id.into_uuid(), code.as_bytes())
        .map_err(|_| NodeError::Storage)?;
    tx.execute(
        "UPDATE enrollment_requests SET approval=?1,result_hash=?2,delivery=?3 WHERE id=?4",
        params![
            bytes(&approval)?,
            hash(&code).as_slice(),
            delivery.as_bytes(),
            id.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let view = pending(&tx, id)?.view(now);
    tx.commit()?;
    Ok(view)
}
pub(crate) fn deny(
    c: &mut Connection,
    token: [u8; 32],
    id: EnrollmentRequestId,
) -> Result<(), NodeError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let session = current_session(&tx, &token, now)?;
    let p = pending(&tx, id)?;
    if p.consumed.is_some() || p.approval.is_some_and(|a| a.user != session.user.id) {
        return Err(NodeError::Forbidden);
    }
    tx.execute(
        "UPDATE enrollment_requests SET denied_at=COALESCE(denied_at,?1),delivery=NULL WHERE id=?2",
        params![now, id.as_uuid().as_bytes().as_slice()],
    )?;
    tx.commit()?;
    Ok(())
}
pub(crate) fn poll(
    c: &mut Connection,
    sealer: &Sealer,
    id: EnrollmentRequestId,
    proof: EnrollmentDeviceProof,
) -> Result<EnrollmentPollResponse, NodeError> {
    let tx = c.transaction()?;
    let now = now()?;
    let p = pending(&tx, id)?;
    p.device(&proof, "poll")?;
    let code = if p.expires > now && p.denied.is_none() && p.consumed.is_none() {
        p.delivery
            .as_ref()
            .map(|d| {
                let plaintext = sealer
                    .open_enrollment(id.into_uuid(), d)
                    .map_err(|_| NodeError::Storage)?;
                Ok::<_, NodeError>(WireSecret::new(
                    std::str::from_utf8(plaintext.as_bytes())
                        .map_err(|_| NodeError::Storage)?
                        .to_owned(),
                ))
            })
            .transpose()?
    } else {
        None
    };
    Ok(EnrollmentPollResponse {
        request: p.view(now),
        code,
        node_id: p.node,
    })
}
pub(crate) fn cancel(
    c: &mut Connection,
    id: EnrollmentRequestId,
    proof: EnrollmentDeviceProof,
) -> Result<(), NodeError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let p = pending(&tx, id)?;
    p.device(&proof, "cancel")?;
    if p.consumed.is_some() {
        return Err(NodeError::Conflict);
    }
    tx.execute(
        "UPDATE enrollment_requests SET denied_at=COALESCE(denied_at,?1),delivery=NULL WHERE id=?2",
        params![now()?, id.as_uuid().as_bytes().as_slice()],
    )?;
    tx.commit()?;
    Ok(())
}
