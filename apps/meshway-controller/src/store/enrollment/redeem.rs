use super::*;
use crate::enrollment::{EnrollmentCommit, NodeAdmission};
use meshway_configuration::NodeIdentity;
use meshway_protocol::node::NodeKeyMetadata;

pub(super) fn existing_keys(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<NodeKeyMetadata, NodeError> {
    let key = |purpose: &str, algorithm| -> Result<BoundNodeKey, NodeError> {
        let (id_bytes,public,generation):(Vec<u8>,Vec<u8>,i64)=tx.query_row("SELECT k.id,k.public_key,k.generation FROM node_keys k JOIN nodes n ON n.network_id=k.network_id AND n.id=k.node_id WHERE k.network_id=?1 AND k.node_id=?2 AND k.purpose=?3 AND k.retired_at IS NULL AND k.revoked_at IS NULL AND n.credentials_revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),purpose],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(NodeError::Unauthorized)?;
        Ok(BoundNodeKey {
            id: id(id_bytes)?,
            generation: Version::new(generation.try_into().map_err(|_| NodeError::Storage)?)
                .map_err(|_| NodeError::Storage)?,
            public_key: NodePublicKey {
                algorithm,
                bytes: Digest::from_bytes(public.try_into().map_err(|_| NodeError::Storage)?),
            },
        })
    };
    Ok(NodeKeyMetadata {
        control: key("control", KeyAlgorithm::Ed25519)?,
        dataplane: key("dataplane", KeyAlgorithm::X25519)?,
    })
}
pub(crate) fn redeem_key_in_transaction(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    request: EnrollmentRequestId,
    secret: WireSecret,
    verifier: WireSecret,
    commit: &dyn EnrollmentCommit,
) -> Result<
    (
        EnrollmentResponse,
        Option<crate::configuration::CommitEffects>,
    ),
    NodeError,
> {
    let now = now()?;
    let p = pending(tx, request)?;
    pkce(&p.challenge, verifier.expose())?;
    if p.denied.is_some()
        || p.cancelled.is_some()
        || p.existing.is_some()
        || p.approval.is_some()
        || secret.expose().len() != 48
        || !secret.expose().starts_with("mwek_")
    {
        return Err(NodeError::Unauthorized);
    }
    let raw=tx.query_row("SELECT id,network_id,tags,initial_settings,created_by,membership_id,expires_at,revoked_at,consumed_request_id FROM enrollment_keys WHERE token_hash=?1",[hash(secret.expose()).as_slice()],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,i64>(6)?,r.get::<_,Option<i64>>(7)?,r.get::<_,Option<Vec<u8>>>(8)?))).optional()?.ok_or(NodeError::Unauthorized)?;
    let key: EnrollmentKeyId = id(raw.0)?;
    let network: NetworkId = id(raw.1)?;
    let actor: UserId = id(raw.4)?;
    if let Some(consumed) = raw.8 {
        if id::<EnrollmentRequestId>(consumed)? != request {
            return Err(NodeError::Conflict);
        }
        let node = p.node.ok_or(NodeError::Storage)?;
        let keys = existing_keys(tx, network, node)?;
        if keys.control.public_key.bytes.as_bytes() != &p.control
            || keys.dataplane.public_key.bytes.as_bytes() != &p.dataplane
        {
            return Err(NodeError::Unauthorized);
        }
        let (_, cfg, bindings) =
            crate::store::execution::current_configuration(tx, sealer, network)?;
        crate::store::execution::identity_eligible(tx, &cfg, &bindings, network, node)?;
        return Ok((
            EnrollmentResponse {
                controller_id: sealer.installation_id(),
                network_id: network,
                node_id: node,
                keys,
            },
            None,
        ));
    }
    if raw.6 <= now || raw.7.is_some() || p.expires <= now || p.node.is_some() {
        return Err(NodeError::Unauthorized);
    }
    let issuer:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships m JOIN users u ON u.id=m.user_id WHERE m.network_id=?1 AND m.id=?2 AND m.user_id=?3 AND m.revoked_at IS NULL AND m.role IN ('owner','admin') AND u.disabled_at IS NULL)",params![network.as_uuid().as_bytes().as_slice(),raw.5,actor.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if !issuer {
        return Err(NodeError::Forbidden);
    }
    let tags: Vec<String> = json(&raw.2)?;
    let (_, cfg, _) = crate::store::execution::current_configuration(tx, sealer, network)?;
    if tags.is_empty() || tags.iter().any(|t| !cfg.tags_owners.contains_key(t)) {
        return Err(NodeError::Forbidden);
    }
    let mut initial: InitialNodeSettings = json(
        sealer
            .open_enrollment(key.into_uuid(), &raw.3)
            .map_err(|_| NodeError::Storage)?
            .as_bytes(),
    )?;
    if initial.hostname.is_none() {
        initial.hostname = Some(p.hostname);
    }
    let node = NodeId::new(uuid()?).map_err(|_| NodeError::Storage)?;
    tx.execute(
        "INSERT INTO nodes(id,network_id,registered_at) VALUES(?1,?2,?3)",
        params![
            node.as_uuid().as_bytes().as_slice(),
            network.as_uuid().as_bytes().as_slice(),
            now
        ],
    )?;
    let keys = crate::store::execution::insert_keys(
        tx,
        network,
        node,
        NodePublicKey {
            algorithm: KeyAlgorithm::Ed25519,
            bytes: p.control.into(),
        },
        NodePublicKey {
            algorithm: KeyAlgorithm::X25519,
            bytes: p.dataplane.into(),
        },
        Version::MIN,
        now,
    )?;
    tx.execute(
        "UPDATE enrollment_requests SET node_id=?1 WHERE id=?2 AND node_id IS NULL",
        params![
            node.as_uuid().as_bytes().as_slice(),
            request.as_uuid().as_bytes().as_slice()
        ],
    )?;
    tx.execute("UPDATE enrollment_keys SET consumed_request_id=?1 WHERE id=?2 AND consumed_request_id IS NULL",params![request.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()])?;
    let admission = NodeAdmission {
        proof: VerifiedEnrollmentProof { request },
        network,
        node,
        request,
        identity: NodeIdentity::Tag { tags },
        membership: None,
        actor,
        initial,
    };
    verify_admission(tx, sealer, &admission)?;
    let effects = commit.insert_node(tx, &admission)?;
    tx.execute(
        "UPDATE enrollment_requests SET node_id=?1,consumed_at=?2,binding_digest=?3 WHERE id=?4",
        params![
            node.as_uuid().as_bytes().as_slice(),
            now,
            p.digest.as_bytes().as_slice(),
            request.as_uuid().as_bytes().as_slice()
        ],
    )?;
    tx.execute("UPDATE enrollment_keys SET consumed_request_id=?1 WHERE id=?2 AND consumed_request_id IS NULL",params![request.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()])?;
    Ok((
        EnrollmentResponse {
            controller_id: sealer.installation_id(),
            network_id: network,
            node_id: node,
            keys,
        },
        Some(effects),
    ))
}
pub(crate) fn redeem_in_transaction(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    id: EnrollmentRequestId,
    request: RedeemEnrollmentRequest,
    commit: &dyn EnrollmentCommit,
) -> Result<
    (
        EnrollmentResponse,
        Option<crate::configuration::CommitEffects>,
    ),
    NodeError,
> {
    let now = now()?;
    let p = pending(tx, id)?;
    p.device(&request.proof, "redeem")?;
    if p.denied.is_some()
        || p.cancelled.is_some()
        || p.result_hash.as_deref() != Some(hash(request.code.expose()).as_slice())
    {
        return Err(NodeError::Unauthorized);
    }
    let approval = p.approval.as_ref().ok_or(NodeError::Unauthorized)?;
    if let Some(node) = p.node {
        // Recover only the exact original identity/key pair. A rotated or revoked
        // admission cannot be recovered by an old enrollment request.
        let keys = existing_keys(tx, approval.network, node)?;
        if keys.control.public_key.bytes.as_bytes() != &p.control
            || keys.dataplane.public_key.bytes.as_bytes() != &p.dataplane
        {
            return Err(NodeError::Unauthorized);
        }
        let (_, cfg, bindings) =
            crate::store::execution::current_configuration(tx, sealer, approval.network)?;
        crate::store::execution::identity_eligible(tx, &cfg, &bindings, approval.network, node)?;
        return Ok((
            EnrollmentResponse {
                controller_id: sealer.installation_id(),
                network_id: approval.network,
                node_id: node,
                keys,
            },
            None,
        ));
    }
    if p.expires <= now {
        return Err(NodeError::Unauthorized);
    }
    let session = current_session(tx, approval.session_hash.as_bytes(), now)?;
    if session.user.id != approval.user {
        return Err(NodeError::Unauthorized);
    }
    let eligible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships WHERE network_id=?1 AND id=?2 AND user_id=?3 AND revoked_at IS NULL)",params![approval.network.as_uuid().as_bytes().as_slice(),approval.membership.as_uuid().as_bytes().as_slice(),approval.user.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if !eligible {
        return Err(NodeError::Forbidden);
    }
    let (node, keys) = if let Some(existing) = &p.existing {
        if existing.network_id != approval.network {
            return Err(NodeError::Forbidden);
        }
        validate_existing(tx, sealer, existing, &p.control, &p.dataplane)?;
        (
            existing.node_id,
            existing_keys(tx, approval.network, existing.node_id)?,
        )
    } else {
        let node = NodeId::new(uuid()?).map_err(|_| NodeError::Storage)?;
        tx.execute(
            "INSERT INTO nodes(id,network_id,registered_at) VALUES(?1,?2,?3)",
            params![
                node.as_uuid().as_bytes().as_slice(),
                approval.network.as_uuid().as_bytes().as_slice(),
                now
            ],
        )?;
        let keys = crate::store::execution::insert_keys(
            tx,
            approval.network,
            node,
            NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: p.control.into(),
            },
            NodePublicKey {
                algorithm: KeyAlgorithm::X25519,
                bytes: p.dataplane.into(),
            },
            Version::MIN,
            now,
        )?;
        (node, keys)
    };
    tx.execute(
        "UPDATE enrollment_requests SET node_id=?1 WHERE id=?2 AND node_id IS NULL",
        params![
            node.as_uuid().as_bytes().as_slice(),
            id.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let admission = NodeAdmission {
        proof: VerifiedEnrollmentProof { request: id },
        network: approval.network,
        node,
        request: id,
        identity: NodeIdentity::User {
            user: session.user.email,
        },
        membership: Some(approval.membership),
        actor: approval.user,
        initial: InitialNodeSettings {
            hostname: Some(p.hostname),
            listeners: Default::default(),
        },
    };
    verify_admission(tx, sealer, &admission)?;
    let effects = commit.insert_node(tx, &admission)?;
    tx.execute("UPDATE enrollment_requests SET node_id=?1,consumed_at=?2,binding_digest=?3,delivery=NULL WHERE id=?4 AND consumed_at IS NULL",params![node.as_uuid().as_bytes().as_slice(),now,p.digest.as_bytes().as_slice(),id.as_uuid().as_bytes().as_slice()])?;
    Ok((
        EnrollmentResponse {
            controller_id: sealer.installation_id(),
            network_id: approval.network,
            node_id: node,
            keys,
        },
        Some(effects),
    ))
}
pub(crate) fn verify_admission(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    admission: &NodeAdmission,
) -> Result<(), NodeError> {
    if admission.proof.request != admission.request {
        return Err(NodeError::Forbidden);
    }
    let now = now()?;
    let p = pending(tx, admission.request)?;
    if p.node != Some(admission.node)
        || p.consumed.is_some()
        || p.denied.is_some()
        || p.cancelled.is_some()
        || p.expires <= now
    {
        return Err(NodeError::Forbidden);
    }
    let keys = existing_keys(tx, admission.network, admission.node)?;
    if keys.control.public_key.bytes.as_bytes() != &p.control
        || keys.dataplane.public_key.bytes.as_bytes() != &p.dataplane
    {
        return Err(NodeError::Forbidden);
    }
    match p.approval {
        Some(a) => {
            let user = current_session(tx, a.session_hash.as_bytes(), now)?.user;
            let eligible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships WHERE network_id=?1 AND id=?2 AND user_id=?3 AND revoked_at IS NULL)",params![a.network.as_uuid().as_bytes().as_slice(),a.membership.as_uuid().as_bytes().as_slice(),a.user.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
            if !eligible
                || a.network != admission.network
                || a.user != admission.actor
                || user.id != a.user
                || admission.membership != Some(a.membership)
                || admission.identity != (NodeIdentity::User { user: user.email })
            {
                return Err(NodeError::Forbidden);
            }
        }
        None => {
            let tags:Vec<u8>=tx.query_row("SELECT k.tags FROM enrollment_keys k JOIN memberships m ON m.network_id=k.network_id AND m.id=k.membership_id AND m.user_id=k.created_by JOIN users u ON u.id=k.created_by WHERE k.network_id=?1 AND k.consumed_request_id=?2 AND k.created_by=?3 AND k.revoked_at IS NULL AND k.expires_at>?4 AND m.revoked_at IS NULL AND m.role IN ('owner','admin') AND u.disabled_at IS NULL",params![admission.network.as_uuid().as_bytes().as_slice(),admission.request.as_uuid().as_bytes().as_slice(),admission.actor.as_uuid().as_bytes().as_slice(),now],|r|r.get(0)).optional()?.ok_or(NodeError::Forbidden)?;
            let tags: Vec<String> = json(&tags)?;
            let (_, cfg, _) = store_configuration(tx, sealer, admission.network)?;
            if tags.iter().any(|t| !cfg.tags_owners.contains_key(t))
                || admission.identity != (NodeIdentity::Tag { tags })
                || admission.membership.is_some()
            {
                return Err(NodeError::Forbidden);
            }
        }
    }
    Ok(())
}
use crate::store::execution::current_configuration as store_configuration;
pub(super) fn validate_existing(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    existing: &ExistingNodeBinding,
    control: &[u8; 32],
    dataplane: &[u8; 32],
) -> Result<(), NodeError> {
    let keys = existing_keys(tx, existing.network_id, existing.node_id)?;
    if keys.control.id != existing.key_id
        || keys.control.generation != existing.key_generation
        || keys.control.public_key.bytes.as_bytes() != control
        || keys.dataplane.public_key.bytes.as_bytes() != dataplane
    {
        return Err(NodeError::Unauthorized);
    }
    let (_, cfg, bindings) = store_configuration(tx, sealer, existing.network_id)?;
    crate::store::execution::identity_eligible(
        tx,
        &cfg,
        &bindings,
        existing.network_id,
        existing.node_id,
    )
}
