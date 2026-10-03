use super::*;
fn key_view(
    tx: &Transaction<'_>,
    network: NetworkId,
    key: EnrollmentKeyId,
) -> Result<EnrollmentKeyView, NodeError> {
    let row=tx.query_row("SELECT tags,created_at,expires_at,revoked_at,consumed_request_id FROM enrollment_keys WHERE network_id=?1 AND id=?2",params![network.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,Option<i64>>(3)?,r.get::<_,Option<Vec<u8>>>(4)?))).optional()?.ok_or(NodeError::NotFound)?;
    Ok(EnrollmentKeyView {
        id: key,
        tags: json(&row.0)?,
        created_at_ms: row.1 as u64,
        expires_at_ms: row.2 as u64,
        revoked_at_ms: row.3.map(|v| v as u64),
        consumed_request_id: row.4.map(id).transpose()?,
    })
}
pub(crate) fn create_key(
    c: &mut Connection,
    sealer: &Sealer,
    token_hash: [u8; 32],
    network: NetworkId,
    request: CreateEnrollmentKeyRequest,
) -> Result<EnrollmentKeyResponse, NodeError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let (user, member) = manager(&tx, token_hash, network, now)?;
    if request.expires_at_ms <= now as u64
        || request.expires_at_ms > now as u64 + 30 * 86_400_000
        || request.tags.is_empty()
        || request.tags.len() > 64
        || request.initial_settings.listeners.len() > 32
    {
        return Err(NodeError::Invalid);
    }
    let (_, cfg, _) = crate::store::execution::current_configuration(&tx, sealer, network)?;
    if request
        .tags
        .iter()
        .any(|t| !cfg.tags_owners.contains_key(t))
    {
        return Err(NodeError::Invalid);
    }
    let digest = sealer.private_digest(b"enrollment-key/create", &bytes(&request)?);
    let previous:Option<(Vec<u8>,Vec<u8>)>=tx.query_row("SELECT id,creation_request_digest FROM enrollment_keys WHERE network_id=?1 AND created_by=?2 AND creation_request_id=?3",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice(),request.request_id.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((key, old)) = previous {
        if old != digest {
            return Err(NodeError::Conflict);
        }
        return Ok(EnrollmentKeyResponse {
            key: key_view(&tx, network, id(key)?)?,
            secret_delivery: SecretDelivery::Unavailable,
            secret: None,
        });
    }
    let key = EnrollmentKeyId::new(uuid()?).map_err(|_| NodeError::Storage)?;
    let secret = format!("mwek_{}", token()?);
    let settings = sealer
        .seal_enrollment(key.into_uuid(), &bytes(&request.initial_settings)?)
        .map_err(|_| NodeError::Storage)?;
    tx.execute("INSERT INTO enrollment_keys(id,network_id,token_hash,tags,initial_settings,created_by,membership_id,created_at,expires_at,creation_request_id,creation_request_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![key.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),hash(&secret).as_slice(),bytes(&request.tags)?,settings.as_bytes(),user.as_uuid().as_bytes().as_slice(),member.as_uuid().as_bytes().as_slice(),now,request.expires_at_ms as i64,request.request_id.as_uuid().as_bytes().as_slice(),digest.as_slice()])?;
    let view = key_view(&tx, network, key)?;
    tx.commit()?;
    Ok(EnrollmentKeyResponse {
        key: view,
        secret_delivery: SecretDelivery::Available,
        secret: Some(WireSecret::new(secret)),
    })
}
pub(crate) fn list_keys(
    c: &mut Connection,
    token: [u8; 32],
    network: NetworkId,
) -> Result<EnrollmentKeysResponse, NodeError> {
    let tx = c.transaction()?;
    manager(&tx, token, network, now()?)?;
    let mut stmt = tx.prepare(
        "SELECT id FROM enrollment_keys WHERE network_id=?1 ORDER BY created_at,id LIMIT 1001",
    )?;
    let ids = stmt
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|v| id::<EnrollmentKeyId>(v?))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.len() > 1000 {
        return Err(NodeError::Busy);
    }
    Ok(EnrollmentKeysResponse {
        items: ids
            .into_iter()
            .map(|id| key_view(&tx, network, id))
            .collect::<Result<_, _>>()?,
    })
}
pub(crate) fn revoke_key(
    c: &mut Connection,
    token: [u8; 32],
    network: NetworkId,
    key: EnrollmentKeyId,
) -> Result<(), NodeError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    manager(&tx, token, network, now)?;
    key_view(&tx, network, key)?;
    tx.execute("UPDATE enrollment_keys SET revoked_at=COALESCE(revoked_at,?1) WHERE network_id=?2 AND id=?3",params![now,network.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()])?;
    tx.commit()?;
    Ok(())
}
