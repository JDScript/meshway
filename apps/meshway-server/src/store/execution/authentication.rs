use super::{bytes, current_configuration, id, identity_eligible, json, now, uuid};
use crate::effect::Effect;
use crate::error::Error;
use crate::execution::NodePrincipal;
use crate::execution::proof;
use crate::secrets::Sealer;
use meshway_model::{
    BoundNodeKey, ChallengeId, Digest, InstanceId, KeyAlgorithm, KeyId, NetworkId, NodeId,
    NodePublicKey, Version,
};
use meshway_protocol::node::*;
use meshway_protocol::{SignedNodeChallengeRequest, challenge_request_transcript};
use rusqlite::{OptionalExtension, Transaction, params};

const CHALLENGE_TTL: i64 = 60_000;
const SESSION_TTL: i64 = 300_000;
const SESSION_ANCHORS: i64 = 4096;
const NODE_SESSION_ANCHORS: i64 = 8;
const CHALLENGES: i64 = 4096;
const NODE_CHALLENGES: i64 = 8;

fn advance_challenge_clock(tx: &Transaction<'_>, now: i64) -> Result<(), Error> {
    let previous: i64 = tx.query_row(
        "SELECT utc_ms FROM node_challenge_clock WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if now < previous {
        return Err(Error::InvalidCredentials);
    }
    // The high-water and all allocation/consumption/GC changes commit together.
    // A rejected or cancelled transaction cannot delete replay evidence while
    // rolling back its time boundary. Never repair a missing clock row here.
    tx.execute(
        "UPDATE node_challenge_clock SET utc_ms=?1 WHERE singleton=1",
        [now],
    )?;
    Ok(())
}

fn cleanup_challenges(tx: &Transaction<'_>, now: i64) -> Result<(), Error> {
    advance_challenge_clock(tx, now)?;
    // A genuine session issuance is a recovery anchor. Other consumed requests
    // retain their original signed-ID lifetime, never usable proof authority.
    tx.execute(
        "DELETE FROM node_challenges WHERE
         (session_anchor=0 AND expires_at<=?1)
         OR (session_anchor=1 AND expires_at<=?1 AND consumed_at<=?2)",
        params![now, now.saturating_sub(SESSION_TTL)],
    )?;
    Ok(())
}

pub(crate) fn authorize_node(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    now: i64,
) -> Result<NodePrincipal, Error> {
    // A recognized, unexpired bearer can receive a precise identity rejection.
    // Missing/expired tokens and malformed proof remain generic credentials
    // failures, which cannot revoke a previously signed offline lease.
    let revoked = tx.query_row(
        "SELECT n.credentials_revoked_at IS NOT NULL OR k.revoked_at IS NOT NULL OR k.retired_at IS NOT NULL FROM node_sessions s JOIN nodes n ON n.network_id=s.network_id AND n.id=s.node_id JOIN node_keys k ON k.id=s.key_id AND k.network_id=s.network_id AND k.node_id=s.node_id WHERE s.token_hash=?1 AND s.boot_id=?2 AND s.created_at<=?3 AND s.expires_at>?3",
        params![token.as_slice(), boot.as_uuid().as_bytes().as_slice(), now],
        |row| row.get::<_, bool>(0),
    ).optional()?;
    if revoked == Some(true) {
        return Err(Error::NodeIdentityRejected);
    }
    let row=tx.query_row("SELECT s.network_id,s.node_id,s.key_id,s.expires_at FROM node_sessions s JOIN nodes n ON n.network_id=s.network_id AND n.id=s.node_id JOIN node_keys k ON k.id=s.key_id AND k.network_id=s.network_id AND k.node_id=s.node_id WHERE s.token_hash=?1 AND s.boot_id=?2 AND s.created_at<=?3 AND s.expires_at>?3 AND n.credentials_revoked_at IS NULL AND k.revoked_at IS NULL AND k.retired_at IS NULL AND k.purpose='control'",params![token.as_slice(),boot.as_uuid().as_bytes().as_slice(),now],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,i64>(3)?))).optional()?.ok_or(Error::InvalidCredentials)?;
    let p = NodePrincipal {
        network: id(row.0)?,
        node: id(row.1)?,
        key: id(row.2)?,
        token_hash: token,
        expires_at_ms: row
            .3
            .try_into()
            .map_err(|_| Error::decode("node_sessions.expires_at"))?,
    };
    let (_, cfg, bindings) = current_configuration(tx, sealer, p.network)?;
    identity_eligible(tx, &cfg, &bindings, p.network, p.node).map_err(|error| match error {
        Error::PermissionDenied | Error::NotFound | Error::InvalidCredentials => {
            Error::NodeIdentityRejected
        }
        error => error,
    })?;
    Ok(p)
}
fn bound_key(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    key: KeyId,
) -> Result<BoundNodeKey, Error> {
    let (key, revoked) = known_key(tx, network, node, key)?;
    if revoked {
        return Err(Error::InvalidCredentials);
    }
    Ok(key)
}

fn known_key(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    key: KeyId,
) -> Result<(BoundNodeKey, bool), Error> {
    let (generation,key_bytes,revoked):(i64,Vec<u8>,bool)=tx.query_row("SELECT k.generation,k.public_key,k.retired_at IS NOT NULL OR k.revoked_at IS NOT NULL OR n.credentials_revoked_at IS NOT NULL FROM node_keys k JOIN nodes n ON n.network_id=k.network_id AND n.id=k.node_id WHERE k.network_id=?1 AND k.node_id=?2 AND k.id=?3 AND k.purpose='control'",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::InvalidCredentials)?;
    Ok((
        BoundNodeKey {
            id: key,
            generation: Version::new(
                generation
                    .try_into()
                    .map_err(|_| Error::decode("node_keys.generation"))?,
            )
            .map_err(|_| Error::decode("node_keys.generation"))?,
            public_key: NodePublicKey {
                algorithm: KeyAlgorithm::Ed25519,
                bytes: Digest::from_bytes(
                    key_bytes
                        .try_into()
                        .map_err(|_| Error::decode("node_keys.public_key"))?,
                ),
            },
        },
        revoked,
    ))
}
pub(crate) fn challenge(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    signed: SignedNodeChallengeRequest,
) -> Result<NodeChallenge, Error> {
    let now = now()?;
    challenge_at(tx, sealer, signed, now)
}

fn challenge_at(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    signed: SignedNodeChallengeRequest,
    now: i64,
) -> Result<NodeChallenge, Error> {
    advance_challenge_clock(tx, now)?;
    let intent = signed.intent;
    let expires = i64::try_from(intent.expires_at_ms).map_err(|_| Error::InvalidCredentials)?;
    if intent.controller_id != sealer.installation_id()
        || expires <= now
        || expires
            > now
                .checked_add(CHALLENGE_TTL)
                .ok_or(Error::InvalidCredentials)?
    {
        return Err(Error::InvalidCredentials);
    }
    let request = &intent.request;
    let (key, revoked) = known_key(tx, request.network_id, request.node_id, request.key_id)?;
    proof::verify_bytes(
        key.public_key.bytes.as_bytes(),
        &challenge_request_transcript(&intent),
        signed.signature.expose(),
    )?;
    if revoked {
        return Err(Error::NodeIdentityRejected);
    }
    let (_, cfg, bindings) = current_configuration(tx, sealer, request.network_id)?;
    identity_eligible(tx, &cfg, &bindings, request.network_id, request.node_id).map_err(
        |error| match error {
            Error::PermissionDenied | Error::NotFound | Error::InvalidCredentials => {
                Error::NodeIdentityRejected
            }
            error => error,
        },
    )?;
    // Authenticate even idempotent reads. A retired key or ineligible identity
    // cannot recover an old challenge. Consumed rows retain their original nonce
    // and are returned here, but consume() still rejects their proof.
    if let Some(existing) = existing_challenge(tx, intent.id, key.generation)? {
        if existing.controller_id != intent.controller_id
            || existing.network_id != request.network_id
            || existing.node_id != request.node_id
            || existing.key_id != request.key_id
            || existing.operation != request.operation
            || existing.request_digest != request.request_digest
            || existing.expires_at_ms != intent.expires_at_ms
        {
            return Err(Error::BindingConflict);
        }
        return Ok(existing);
    }
    // Nothing above allocates or cleans quota state. In particular anonymous
    // requests knowing a Node ID cannot reserve its eight pending proof slots.
    cleanup_challenges(tx, now)?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM node_challenges WHERE consumed_at IS NULL AND expires_at>?1",
        [now],
        |r| r.get(0),
    )?;
    let own: i64 = tx.query_row(
        "SELECT count(*) FROM node_challenges WHERE network_id=?1 AND node_id=?2 AND consumed_at IS NULL AND expires_at>?3",
        params![request.network_id.as_uuid().as_bytes().as_slice(), request.node_id.as_uuid().as_bytes().as_slice(), now],
        |r| r.get(0),
    )?;
    // Recovery proofs also retain signed-ID tombstones until the request expiry.
    // Bound those rows in addition to the pending and session-anchor budgets.
    let (retained, own_retained): (i64, i64) = tx.query_row(
        "SELECT count(*),coalesce(sum(network_id=?1 AND node_id=?2),0) FROM node_challenges",
        params![
            request.network_id.as_uuid().as_bytes().as_slice(),
            request.node_id.as_uuid().as_bytes().as_slice()
        ],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if count >= CHALLENGES
        || own >= NODE_CHALLENGES
        || retained >= CHALLENGES + SESSION_ANCHORS
        || own_retained >= NODE_CHALLENGES + NODE_SESSION_ANCHORS
    {
        return Err(Error::Busy);
    }
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::ENTROPY)?;
    let c = NodeChallenge {
        id: intent.id,
        controller_id: sealer.installation_id(),
        network_id: request.network_id,
        node_id: request.node_id,
        key_id: request.key_id,
        key_generation: key.generation,
        nonce: nonce.into(),
        operation: request.operation,
        request_digest: request.request_digest,
        expires_at_ms: intent.expires_at_ms,
    };
    tx.execute("INSERT INTO node_challenges(id,controller_id,network_id,node_id,key_id,nonce,request_digest,operation,created_at,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![c.id.as_uuid().as_bytes().as_slice(),c.controller_id.as_uuid().as_bytes().as_slice(),c.network_id.as_uuid().as_bytes().as_slice(),c.node_id.as_uuid().as_bytes().as_slice(),c.key_id.as_uuid().as_bytes().as_slice(),nonce.as_slice(),c.request_digest.as_bytes().as_slice(),operation(c.operation),now,expires])?;
    Ok(c)
}

fn existing_challenge(
    tx: &Transaction<'_>,
    challenge: ChallengeId,
    generation: Version,
) -> Result<Option<NodeChallenge>, Error> {
    let mut statement = tx.prepare(
        "SELECT controller_id,network_id,node_id,key_id,nonce,request_digest,operation,expires_at
         FROM node_challenges WHERE id=?1",
    )?;
    let mut rows = statement.query([challenge.as_uuid().as_bytes().as_slice()])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let nonce: Vec<u8> = row.get(4)?;
    let digest: Vec<u8> = row.get(5)?;
    let op: String = row.get(6)?;
    let expires: i64 = row.get(7)?;
    Ok(Some(NodeChallenge {
        id: challenge,
        controller_id: id(row.get(0)?)?,
        network_id: id(row.get(1)?)?,
        node_id: id(row.get(2)?)?,
        key_id: id(row.get(3)?)?,
        key_generation: generation,
        nonce: Digest::from_bytes(
            nonce
                .try_into()
                .map_err(|_| Error::decode("node_challenges.nonce"))?,
        ),
        request_digest: Digest::from_bytes(
            digest
                .try_into()
                .map_err(|_| Error::decode("node_challenges.request_digest"))?,
        ),
        operation: match op.as_str() {
            "session" => ProofOperation::Session,
            "rotate" => ProofOperation::Rotate,
            _ => return Err(Error::decode("node_challenges.operation")),
        },
        expires_at_ms: expires
            .try_into()
            .map_err(|_| Error::decode("node_challenges.expires_at"))?,
    }))
}

fn recover_session(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    fresh: &NodeChallenge,
    now: i64,
) -> Result<Option<NodeSessionResponse>, Error> {
    // Bounded by the per-node retained-row budget. Include other intents and
    // keys so deleted/expired sessions and retired generations release their
    // orphaned anchor slots before checking capacity for a new issuance.
    let mut stmt = tx.prepare(
        "SELECT c.id,c.controller_id,c.key_id,k.generation,c.nonce,c.request_digest,
                c.expires_at,c.consumed_at
         FROM node_challenges c JOIN node_keys k
           ON k.network_id=c.network_id AND k.node_id=c.node_id AND k.id=c.key_id
         WHERE c.network_id=?1 AND c.node_id=?2 AND c.operation='session'
            AND c.session_anchor=1 AND c.id!=?3
         ORDER BY c.consumed_at,c.id LIMIT ?4",
    )?;
    let mut rows = stmt.query(params![
        fresh.network_id.as_uuid().as_bytes().as_slice(),
        fresh.node_id.as_uuid().as_bytes().as_slice(),
        fresh.id.as_uuid().as_bytes().as_slice(),
        NODE_SESSION_ANCHORS + NODE_CHALLENGES
    ])?;
    let mut anchors = Vec::new();
    while let Some(row) = rows.next()? {
        let nonce: Vec<u8> = row.get(4)?;
        let digest: Vec<u8> = row.get(5)?;
        let generation: i64 = row.get(3)?;
        let expires: i64 = row.get(6)?;
        anchors.push((
            NodeChallenge {
                id: id(row.get(0)?)?,
                controller_id: id(row.get(1)?)?,
                key_id: id(row.get(2)?)?,
                key_generation: Version::new(
                    generation
                        .try_into()
                        .map_err(|_| Error::decode("node_keys.generation"))?,
                )
                .map_err(|_| Error::decode("node_keys.generation"))?,
                nonce: Digest::from_bytes(
                    nonce
                        .try_into()
                        .map_err(|_| Error::decode("node_challenges.nonce"))?,
                ),
                request_digest: Digest::from_bytes(
                    digest
                        .try_into()
                        .map_err(|_| Error::decode("node_challenges.request_digest"))?,
                ),
                expires_at_ms: expires
                    .try_into()
                    .map_err(|_| Error::decode("node_challenges.expires_at"))?,
                ..fresh.clone()
            },
            row.get::<_, i64>(7)?,
        ));
    }
    drop(rows);
    drop(stmt);
    for (anchor, consumed_at) in anchors {
        let (token, hash) = proof::session_token(sealer, boot, &anchor)?;
        let expires: Option<i64> = tx
            .query_row(
                "SELECT s.expires_at FROM node_sessions s JOIN node_keys k
               ON k.network_id=s.network_id AND k.node_id=s.node_id AND k.id=s.key_id
             WHERE s.token_hash=?1 AND s.boot_id=?2 AND s.network_id=?3 AND s.node_id=?4
               AND s.key_id=?5 AND s.created_at=?6 AND s.created_at<=?7 AND s.expires_at>?7
               AND k.revoked_at IS NULL AND k.retired_at IS NULL",
                params![
                    hash.as_slice(),
                    boot.as_uuid().as_bytes().as_slice(),
                    anchor.network_id.as_uuid().as_bytes().as_slice(),
                    anchor.node_id.as_uuid().as_bytes().as_slice(),
                    anchor.key_id.as_uuid().as_bytes().as_slice(),
                    consumed_at,
                    now
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(expires) = expires {
            if anchor.controller_id == fresh.controller_id
                && anchor.key_id == fresh.key_id
                && anchor.key_generation == fresh.key_generation
                && anchor.request_digest == fresh.request_digest
            {
                // Keep the consumed fresh row: deleting it would let the same
                // still-valid signed allocation request mint another nonce.
                return Ok(Some(NodeSessionResponse {
                    node_id: fresh.node_id,
                    network_id: fresh.network_id,
                    key_id: fresh.key_id,
                    token,
                    expires_at_ms: expires
                        .try_into()
                        .map_err(|_| Error::decode("node_sessions.expires_at"))?,
                }));
            }
        } else {
            // A missing row cannot be recreated from an old anchor: doing so
            // would reactivate a captured expired token. Preserve a still-live
            // signed allocation ID, but release its session-anchor budget.
            let sql = if anchor.expires_at_ms <= now as u64 {
                "DELETE FROM node_challenges WHERE id=?1"
            } else {
                "UPDATE node_challenges SET session_anchor=0 WHERE id=?1"
            };
            tx.execute(sql, [anchor.id.as_uuid().as_bytes().as_slice()])?;
        }
    }
    Ok(None)
}
fn operation(op: ProofOperation) -> &'static str {
    match op {
        ProofOperation::Session => "session",
        ProofOperation::Rotate => "rotate",
    }
}
fn consume(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodeProof,
    op: ProofOperation,
    digest: Digest,
    now: i64,
) -> Result<NodeChallenge, Error> {
    advance_challenge_clock(tx, now)?;
    let row=tx.query_row("SELECT network_id,node_id,key_id,nonce,request_digest,expires_at FROM node_challenges WHERE id=?1 AND controller_id=?2 AND operation=?3 AND consumed_at IS NULL AND expires_at>?4 AND created_at<=?4",params![p.challenge_id.as_uuid().as_bytes().as_slice(),sealer.installation_id().as_uuid().as_bytes().as_slice(),operation(op),now],|r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?,r.get::<_,Vec<u8>>(3)?,r.get::<_,Vec<u8>>(4)?,r.get::<_,i64>(5)?))).optional()?.ok_or(Error::InvalidCredentials)?;
    if row.4 != digest.as_bytes() {
        return Err(Error::InvalidCredentials);
    }
    let (network, node, key) = (id(row.0)?, id(row.1)?, id(row.2)?);
    let key = bound_key(tx, network, node, key)?;
    let c = NodeChallenge {
        id: p.challenge_id,
        controller_id: sealer.installation_id(),
        network_id: network,
        node_id: node,
        key_id: key.id,
        key_generation: key.generation,
        nonce: Digest::from_bytes(
            row.3
                .try_into()
                .map_err(|_| Error::decode("node_challenges.nonce"))?,
        ),
        operation: op,
        request_digest: digest,
        expires_at_ms: row
            .5
            .try_into()
            .map_err(|_| Error::decode("node_challenges.expires_at"))?,
    };
    proof::verify(&key.public_key, &c, p.signature.expose())?;
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    identity_eligible(tx, &cfg, &bindings, network, node)?;
    tx.execute(
        "UPDATE node_challenges SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
        params![now, c.id.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(c)
}
pub(crate) fn session(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    request: NodeSessionRequest,
) -> Result<NodeSessionResponse, Error> {
    let now = now()?;
    let c = consume(
        tx,
        sealer,
        &request.proof,
        ProofOperation::Session,
        proof::request_digest(&request.intent)?,
        now,
    )?;
    tx.execute(
        "DELETE FROM node_sessions WHERE expires_at<=?1 OR boot_id!=?2",
        params![now, boot.as_uuid().as_bytes().as_slice()],
    )?;
    cleanup_challenges(tx, now)?;
    if let Some(response) = recover_session(tx, sealer, boot, &c, now)? {
        return Ok(response);
    }
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM node_sessions WHERE node_id=?1",
        [c.node_id.as_uuid().as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    let (anchors, own): (i64, i64) = tx.query_row(
        "SELECT count(*),coalesce(sum(node_id=?1),0) FROM node_challenges
         WHERE session_anchor=1 AND id!=?2",
        params![
            c.node_id.as_uuid().as_bytes().as_slice(),
            c.id.as_uuid().as_bytes().as_slice()
        ],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if count >= 8 || anchors >= SESSION_ANCHORS || own >= NODE_SESSION_ANCHORS {
        return Err(Error::Busy);
    }
    let (token, hash) = proof::session_token(sealer, boot, &c)?;
    let expires = now + SESSION_TTL;
    tx.execute("INSERT INTO node_sessions(token_hash,network_id,node_id,key_id,boot_id,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![hash.as_slice(),c.network_id.as_uuid().as_bytes().as_slice(),c.node_id.as_uuid().as_bytes().as_slice(),c.key_id.as_uuid().as_bytes().as_slice(),boot.as_uuid().as_bytes().as_slice(),now,expires])?;
    tx.execute(
        "UPDATE node_challenges SET session_anchor=1 WHERE id=?1",
        [c.id.as_uuid().as_bytes().as_slice()],
    )?;
    Ok(NodeSessionResponse {
        node_id: c.node_id,
        network_id: c.network_id,
        key_id: c.key_id,
        token,
        expires_at_ms: expires as u64,
    })
}
pub(crate) fn rotate(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    request: NodeRotationRequest,
) -> Result<(NodeKeyMetadata, Vec<Effect>), Error> {
    let now = now()?;
    let digest = proof::request_digest(&request.intent)?;
    // A response can be lost after retirement of the old signing key. Recover
    // only the recorded result, authenticated with the newly bound private key.
    let recovery: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT request_digest,challenge,result FROM node_rotations WHERE challenge_id=?1",
            [request
                .old_proof
                .challenge_id
                .as_uuid()
                .as_bytes()
                .as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((stored_digest, challenge, result)) = recovery {
        if stored_digest != digest.as_bytes() {
            return Err(Error::BindingConflict);
        }
        let challenge: NodeChallenge = json(&challenge)?;
        let keys: NodeKeyMetadata = json(&result)?;
        let current = bound_key(tx, challenge.network_id, challenge.node_id, keys.control.id)?;
        if current != keys.control {
            return Err(Error::InvalidCredentials);
        }
        let dataplane_current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM node_keys WHERE network_id=?1 AND node_id=?2
             AND id=?3 AND purpose='dataplane' AND generation=?4 AND retired_at IS NULL AND revoked_at IS NULL)",
            params![challenge.network_id.as_uuid().as_bytes().as_slice(), challenge.node_id.as_uuid().as_bytes().as_slice(), keys.dataplane.id.as_uuid().as_bytes().as_slice(), keys.dataplane.generation.get() as i64],
            |row| row.get(0),
        )?;
        if !dataplane_current {
            return Err(Error::InvalidCredentials);
        }
        let (_, cfg, bindings) = current_configuration(tx, sealer, challenge.network_id)?;
        identity_eligible(tx, &cfg, &bindings, challenge.network_id, challenge.node_id)?;
        proof::verify(
            &current.public_key,
            &challenge,
            request.new_signature.expose(),
        )?;
        return Ok((keys, Vec::new()));
    }
    let c = consume(
        tx,
        sealer,
        &request.old_proof,
        ProofOperation::Rotate,
        digest,
        now,
    )?;
    let used: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM node_rotations WHERE network_id=?1 AND node_id=?2 AND request_id=?3)",
        params![c.network_id.as_uuid().as_bytes().as_slice(), c.node_id.as_uuid().as_bytes().as_slice(), request.intent.request_id.as_uuid().as_bytes().as_slice()],
        |row| row.get(0),
    )?;
    if used {
        return Err(Error::BindingConflict);
    }
    proof::verify(
        &request.intent.control_key,
        &c,
        request.new_signature.expose(),
    )?;
    if request.intent.dataplane_key.algorithm != KeyAlgorithm::X25519
        || request.intent.dataplane_key.bytes.as_bytes() == &[0; 32]
    {
        return Err(Error::InvalidRequest);
    }
    tx.execute(
        "UPDATE node_keys SET retired_at=?1 WHERE network_id=?2 AND node_id=?3
         AND retired_at IS NULL AND revoked_at IS NULL",
        params![
            now,
            c.network_id.as_uuid().as_bytes().as_slice(),
            c.node_id.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let generation =
        Version::new(c.key_generation.get() + 1).map_err(|_| Error::BindingConflict)?;
    let keys = insert_keys(
        tx,
        c.network_id,
        c.node_id,
        request.intent.control_key,
        request.intent.dataplane_key,
        generation,
        now,
    )?;
    tx.execute(
        "INSERT INTO node_rotations(network_id,node_id,request_id,request_digest,challenge_id,
         challenge,control_key_id,dataplane_key_id,generation,result)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            c.network_id.as_uuid().as_bytes().as_slice(),
            c.node_id.as_uuid().as_bytes().as_slice(),
            request.intent.request_id.as_uuid().as_bytes().as_slice(),
            digest.as_bytes().as_slice(),
            c.id.as_uuid().as_bytes().as_slice(),
            bytes(&c)?,
            keys.control.id.as_uuid().as_bytes().as_slice(),
            keys.dataplane.id.as_uuid().as_bytes().as_slice(),
            generation.get() as i64,
            bytes(&keys)?
        ],
    )?;
    let netmap = crate::store::netmap::recompile(
        tx,
        sealer,
        c.network_id,
        now,
        crate::store::netmap::Trigger::Node,
    )?;
    let mut effects = vec![
        Effect::Authorization {
            network_id: c.network_id,
        },
        Effect::Progress {
            network_id: c.network_id,
        },
    ];
    effects.extend(netmap.effects());
    Ok((keys, effects))
}
pub(crate) fn insert_keys(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    control: NodePublicKey,
    dataplane: NodePublicKey,
    generation: Version,
    now: i64,
) -> Result<NodeKeyMetadata, Error> {
    let make = |purpose: &str,
                algorithm: &str,
                public_key: NodePublicKey|
     -> Result<BoundNodeKey, Error> {
        let id = KeyId::new(uuid()?).map_err(|_| Error::Internal("generated node key ID"))?;
        tx.execute(
            "INSERT INTO node_keys(id,network_id,node_id,purpose,algorithm,public_key,generation,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![id.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice(),purpose,algorithm,public_key.bytes.as_bytes().as_slice(),
                generation.get() as i64,now],
        )?;
        Ok(BoundNodeKey {
            id,
            generation,
            public_key,
        })
    };
    Ok(NodeKeyMetadata {
        control: make("control", "ed25519", control)?,
        dataplane: make("dataplane", "x25519", dataplane)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn challenge_gc_and_high_water_commit_together_and_rollback_stays_fenced_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clock.sqlite");
        let mut db = Connection::open(&path).unwrap();
        crate::store::schema::tests::initialize_fixture(&mut db).unwrap();
        let tx = db.transaction().unwrap();
        advance_challenge_clock(&tx, 1000).unwrap();
        // Storage-only retention fixture. Signature/identity admission is tested
        // separately through the real enrolled Server HTTP fixture.
        let user = [6u8; 16];
        let network = [3u8; 16];
        let node = [4u8; 16];
        let key = [5u8; 16];
        // Keep the real foreign keys enabled and seed their complete metadata
        // graph. Opaque sealed values are never opened by this GC-only test.
        tx.execute(
            "INSERT INTO users(id,email,password_hash,credential_version,created_at)
             VALUES(?1,'clock@example.test','$argon2id$v=19$storage-fixture',1,0)",
            [user.as_slice()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO networks(id,name,created_by,created_at,creation_request_id,creation_request_digest,current_revision,publication_sequence)
             VALUES(?1,'Clock fixture',?2,0,?3,zeroblob(32),1,0)",
            params![network.as_slice(), user.as_slice(), [7u8; 16].as_slice()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO configuration_versions(network_id,revision,source_jsonc,configuration,bindings,created_by,created_at,cause,request_id)
             VALUES(?1,1,zeroblob(61),zeroblob(61),zeroblob(61),?2,0,'networkCreated',?3)",
            params![network.as_slice(), user.as_slice(), [7u8; 16].as_slice()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO nodes(id,network_id,registered_at) VALUES(?1,?2,0)",
            params![node.as_slice(), network.as_slice()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO node_keys(id,network_id,node_id,purpose,algorithm,public_key,generation,created_at)
             VALUES(?1,?2,?3,'control','ed25519',?4,1,0)",
            params![key.as_slice(), network.as_slice(), node.as_slice(), [8u8; 32].as_slice()],
        )
        .unwrap();
        tx.execute("INSERT INTO node_challenges(id,controller_id,network_id,node_id,key_id,nonce,request_digest,operation,created_at,expires_at) VALUES(x'01010101010101010101010101010101',x'02020202020202020202020202020202',x'03030303030303030303030303030303',x'04040404040404040404040404040404',x'05050505050505050505050505050505',zeroblob(32),zeroblob(32),'session',1000,1100)", []).unwrap();
        tx.commit().unwrap();
        {
            let tx = db.transaction().unwrap();
            cleanup_challenges(&tx, 1100).unwrap();
            assert_eq!(
                tx.query_row("SELECT count(*) FROM node_challenges", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            tx.rollback().unwrap();
        }
        assert_eq!(
            db.query_row("SELECT utc_ms FROM node_challenge_clock", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1000
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM node_challenges", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let tx = db.transaction().unwrap();
        cleanup_challenges(&tx, 1100).unwrap();
        tx.commit().unwrap();
        drop(db);
        let mut db = Connection::open(&path).unwrap();
        crate::store::schema::tests::initialize_fixture(&mut db).unwrap();
        let tx = db.transaction().unwrap();
        // These are the very same admission/GC boundaries used by challenge_at
        // and consume. A previously expired signed request cannot reach lookup
        // or nonce allocation even after its old row has actually been deleted.
        assert_eq!(
            advance_challenge_clock(&tx, 1050),
            Err(Error::InvalidCredentials)
        );
        assert_eq!(
            cleanup_challenges(&tx, 1050),
            Err(Error::InvalidCredentials)
        );
        assert_eq!(
            tx.query_row("SELECT utc_ms FROM node_challenge_clock", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1100
        );
        assert_eq!(
            tx.query_row("SELECT count(*) FROM node_challenges", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        tx.rollback().unwrap();
    }
}
