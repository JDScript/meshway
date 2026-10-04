use super::{
    bytes, current_configuration, id, identity_eligible, json, now, publications::revoke_execution,
    uuid,
};
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
use rusqlite::{OptionalExtension, Transaction, params};

const CHALLENGE_TTL: i64 = 60_000;
const SESSION_TTL: i64 = 300_000;
const SESSION_ANCHORS: i64 = 4096;
const NODE_SESSION_ANCHORS: i64 = 8;

fn cleanup_challenges(tx: &Transaction<'_>, now: i64) -> Result<(), Error> {
    // A consumed session challenge is a recovery anchor, never a usable proof.
    // Rotation and unconsumed challenges retain their original proof lifetime.
    tx.execute(
        "DELETE FROM node_challenges WHERE
         ((consumed_at IS NULL OR operation!='session') AND expires_at<=?1)
         OR (operation='session' AND consumed_at IS NOT NULL AND consumed_at<=?2)",
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
    identity_eligible(tx, &cfg, &bindings, p.network, p.node)?;
    Ok(p)
}
fn bound_key(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    key: KeyId,
) -> Result<BoundNodeKey, Error> {
    let (generation,key_bytes):(i64,Vec<u8>)=tx.query_row("SELECT k.generation,k.public_key FROM node_keys k JOIN nodes n ON n.network_id=k.network_id AND n.id=k.node_id WHERE k.network_id=?1 AND k.node_id=?2 AND k.id=?3 AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL AND n.credentials_revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),key.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::InvalidCredentials)?;
    Ok(BoundNodeKey {
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
    })
}
pub(crate) fn challenge(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    request: NodeChallengeRequest,
) -> Result<NodeChallenge, Error> {
    let now = now()?;
    let key = bound_key(tx, request.network_id, request.node_id, request.key_id)?;
    let (_, cfg, bindings) = current_configuration(tx, sealer, request.network_id)?;
    identity_eligible(tx, &cfg, &bindings, request.network_id, request.node_id)?;
    cleanup_challenges(tx, now)?;
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM node_challenges WHERE consumed_at IS NULL AND expires_at>?1",
        [now],
        |r| r.get(0),
    )?;
    let own: i64 = tx.query_row(
        "SELECT count(*) FROM node_challenges WHERE node_id=?1 AND consumed_at IS NULL AND expires_at>?2",
        params![request.node_id.as_uuid().as_bytes().as_slice(), now],
        |r| r.get(0),
    )?;
    if count >= 4096 || own >= 8 {
        return Err(Error::Busy);
    }
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|_| Error::ENTROPY)?;
    let c = NodeChallenge {
        id: ChallengeId::new(uuid()?).map_err(|_| Error::Internal("generated challenge ID"))?,
        controller_id: sealer.installation_id(),
        network_id: request.network_id,
        node_id: request.node_id,
        key_id: request.key_id,
        key_generation: key.generation,
        nonce: nonce.into(),
        operation: request.operation,
        request_digest: request.request_digest,
        expires_at_ms: (now + CHALLENGE_TTL) as u64,
    };
    tx.execute("INSERT INTO node_challenges(id,controller_id,network_id,node_id,key_id,nonce,request_digest,operation,created_at,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![c.id.as_uuid().as_bytes().as_slice(),c.controller_id.as_uuid().as_bytes().as_slice(),c.network_id.as_uuid().as_bytes().as_slice(),c.node_id.as_uuid().as_bytes().as_slice(),c.key_id.as_uuid().as_bytes().as_slice(),nonce.as_slice(),c.request_digest.as_bytes().as_slice(),operation(c.operation),now,now+CHALLENGE_TTL])?;
    Ok(c)
}

fn recover_session(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    fresh: &NodeChallenge,
    now: i64,
) -> Result<Option<NodeSessionResponse>, Error> {
    // Bounded by the separate per-node anchor budget. Include other intents and
    // keys so deleted/expired sessions and retired generations release their
    // orphaned anchor slots before checking capacity for a new issuance.
    let mut stmt = tx.prepare(
        "SELECT c.id,c.controller_id,c.key_id,k.generation,c.nonce,c.request_digest,
                c.expires_at,c.consumed_at
         FROM node_challenges c JOIN node_keys k
           ON k.network_id=c.network_id AND k.node_id=c.node_id AND k.id=c.key_id
         WHERE c.network_id=?1 AND c.node_id=?2 AND c.operation='session'
           AND c.consumed_at IS NOT NULL AND c.id!=?3
         ORDER BY c.consumed_at,c.id LIMIT ?4",
    )?;
    let mut rows = stmt.query(params![
        fresh.network_id.as_uuid().as_bytes().as_slice(),
        fresh.node_id.as_uuid().as_bytes().as_slice(),
        fresh.id.as_uuid().as_bytes().as_slice(),
        NODE_SESSION_ANCHORS
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
                // The duplicate fresh proof was consumed and verified above.
                // Deletion makes replay invalid without accumulating anchors.
                tx.execute(
                    "DELETE FROM node_challenges WHERE id=?1",
                    [fresh.id.as_uuid().as_bytes().as_slice()],
                )?;
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
            // would reactivate a captured expired token. New issuance uses fresh.
            tx.execute(
                "DELETE FROM node_challenges WHERE id=?1",
                [anchor.id.as_uuid().as_bytes().as_slice()],
            )?;
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
         WHERE operation='session' AND consumed_at IS NOT NULL AND id!=?2",
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
    revoke_execution(tx, sealer, c.network_id, c.node_id, now)?;
    Ok((
        keys,
        vec![
            Effect::Authorization {
                network_id: c.network_id,
            },
            Effect::Progress {
                network_id: c.network_id,
            },
        ],
    ))
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
