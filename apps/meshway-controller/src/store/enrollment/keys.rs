use super::*;
fn key_view(
    tx: &Transaction<'_>,
    network: NetworkId,
    key: EnrollmentKeyId,
    sealer: &Sealer,
    now: i64,
) -> Result<EnrollmentKeyView, NodeError> {
    let row = tx
        .query_row(
            "SELECT k.tags,k.created_at,k.expires_at,k.revoked_at,k.consumed_request_id,
                k.created_by,k.initial_settings,r.consumed_at,r.node_id
         FROM enrollment_keys k LEFT JOIN enrollment_requests r ON r.id=k.consumed_request_id
         WHERE k.network_id=?1 AND k.id=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                key.as_uuid().as_bytes().as_slice()
            ],
            |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<Vec<u8>>>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, Vec<u8>>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, Option<Vec<u8>>>(8)?,
                ))
            },
        )
        .optional()?
        .ok_or(NodeError::NotFound)?;
    let settings: InitialNodeSettings = json(
        sealer
            .open_enrollment(key.into_uuid(), &row.6)
            .map_err(|_| NodeError::Storage)?
            .as_bytes(),
    )?;
    Ok(EnrollmentKeyView {
        created_by: id(row.5)?,
        initial_settings: InitialNodeSettingsView {
            hostname: settings.hostname,
            listeners: settings
                .listeners
                .iter()
                .map(|(name, value)| (name.clone(), value.into()))
                .collect(),
        },
        actions: EnrollmentKeyActions {
            revoke: row.3.is_none() && row.4.is_none() && row.2 > now,
        },
        consumed_at_ms: row.7.map(|v| v as u64),
        node_id: row.8.map(id).transpose()?,
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
            key: key_view(&tx, network, id(key)?, sealer, now)?,
            secret_delivery: SecretDelivery::Unavailable,
            secret: None,
        });
    }
    let mut initial_settings = request.initial_settings;
    crate::enrollment::settings::normalize(&tx, network, &mut initial_settings, sealer)?;
    let key = EnrollmentKeyId::new(uuid()?).map_err(|_| NodeError::Storage)?;
    let secret = format!("mwek_{}", token()?);
    let settings = sealer
        .seal_enrollment(key.into_uuid(), &bytes(&initial_settings)?)
        .map_err(|_| NodeError::Storage)?;
    tx.execute("INSERT INTO enrollment_keys(id,network_id,token_hash,tags,initial_settings,created_by,membership_id,created_at,expires_at,creation_request_id,creation_request_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![key.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),hash(&secret).as_slice(),bytes(&request.tags)?,settings.as_bytes(),user.as_uuid().as_bytes().as_slice(),member.as_uuid().as_bytes().as_slice(),now,request.expires_at_ms as i64,request.request_id.as_uuid().as_bytes().as_slice(),digest.as_slice()])?;
    let view = key_view(&tx, network, key, sealer, now)?;
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
    limit: u16,
    cursor: Option<String>,
    sealer: &Sealer,
) -> Result<EnrollmentKeysResponse, NodeError> {
    use crate::{networks::pagination::Window, secrets::CursorScope};
    let tx = c.transaction()?;
    let now = now()?;
    let (user, _) = manager(&tx, token, network, now)?;
    let scope = CursorScope::EnrollmentKeys { user, network };
    let upper = tx.query_row(
        "SELECT coalesce(max(sequence),0) FROM enrollment_keys",
        [],
        |r| r.get(0),
    )?;
    let mut window = match cursor {
        Some(cursor) => {
            Window::decode(sealer, scope, &cursor, now).map_err(|_| NodeError::InvalidCursor)?
        }
        None => Window::new(upper, 0, now).map_err(|_| NodeError::Storage)?,
    };
    let total: i64 = tx.query_row(
        "SELECT count(*) FROM enrollment_keys WHERE network_id=?1 AND sequence<=?2",
        params![network.as_uuid().as_bytes().as_slice(), window.upper],
        |r| r.get(0),
    )?;
    let mut stmt = tx.prepare(
        "SELECT sequence,id FROM enrollment_keys WHERE network_id=?1 AND sequence<=?2 ORDER BY sequence DESC LIMIT ?3",
    )?;
    let mut ids = stmt
        .query_map(
            params![
                network.as_uuid().as_bytes().as_slice(),
                window.upper - window.last,
                i64::from(limit) + 1
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let more = ids.len() > usize::from(limit);
    ids.truncate(usize::from(limit));
    if let Some((sequence, _)) = ids.last() {
        window.last = window.upper - sequence + 1;
    }
    let result = EnrollmentKeysResponse {
        total: Count::new(total.try_into().map_err(|_| NodeError::Storage)?)
            .map_err(|_| NodeError::Storage)?,
        as_of: window.as_of().map_err(|_| NodeError::Storage)?,
        next_cursor: if more {
            Some(
                window
                    .encode(sealer, scope)
                    .map_err(|_| NodeError::Storage)?,
            )
        } else {
            None
        },
        items: ids
            .into_iter()
            .map(|(_, key)| key_view(&tx, network, id(key)?, sealer, now))
            .collect::<Result<_, _>>()?,
    };
    drop(stmt);
    tx.commit()?;
    Ok(result)
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
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM enrollment_keys WHERE network_id=?1 AND id=?2)",
        params![
            network.as_uuid().as_bytes().as_slice(),
            key.as_uuid().as_bytes().as_slice()
        ],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(NodeError::NotFound);
    }
    tx.execute("UPDATE enrollment_keys SET revoked_at=COALESCE(revoked_at,?1) WHERE network_id=?2 AND id=?3 AND consumed_request_id IS NULL AND expires_at>?1",params![now,network.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()])?;
    tx.commit()?;
    Ok(())
}
