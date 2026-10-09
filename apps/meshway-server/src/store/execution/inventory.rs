use super::{current_configuration, id, now, publications::revoke_execution, user_binding};
use crate::effect::Effect;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_model::{
    BoundNodeKey, ConfigurationRef, Digest, InstanceId, KeyAlgorithm, MembershipRole, NetworkId,
    NodeId, NodePublicKey, PublicationId, Revision, Sequence, Version,
};
use meshway_protocol::{execution::*, node::*};
use rusqlite::{OptionalExtension, Transaction, params};
use std::collections::BTreeMap;
/// Marks one key generation as permanently revoked. Returns whether the key
/// was still current; only then does the Node lose its execution. A retired
/// generation can still be revoked so old material can never be recovered;
/// a generation that is already revoked reports `StateConflict`.
pub(crate) fn revoke_key(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
    node: NodeId,
    request: RevokeNodeKeyRequest,
) -> Result<(bool, Vec<Effect>), Error> {
    let now = now()?;
    let user = super::super::authorization::current_session(tx, &token, now)?
        .user
        .id;
    crate::store::authorization::authorize_manager(tx, network, user).map_err(
        |error| match error {
            Error::PermissionDenied | Error::NotFound => Error::PermissionDenied,
            other => other,
        },
    )?;
    let state: Option<(bool, bool)> = tx
        .query_row(
            "SELECT retired_at IS NULL, revoked_at IS NULL FROM node_keys WHERE network_id=?1 AND node_id=?2 AND id=?3 AND generation=?4",
            params![
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice(),
                request.key_id.as_uuid().as_bytes().as_slice(),
                request.expected_generation.get() as i64
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((current, unrevoked)) = state else {
        return Err(Error::VersionConflict);
    };
    if !unrevoked {
        return Err(Error::StateConflict);
    }
    tx.execute(
        "UPDATE node_keys SET revoked_at=?1 WHERE network_id=?2 AND node_id=?3 AND id=?4 AND revoked_at IS NULL",
        params![
            now,
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice(),
            request.key_id.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let mut effects = Vec::new();
    if current {
        revoke_execution(tx, sealer, network, node, now)?;
        let netmap = crate::store::netmap::recompile(tx, sealer, network, now)?;
        effects.extend([
            Effect::Authorization {
                network_id: network,
            },
            Effect::Progress {
                network_id: network,
            },
        ]);
        effects.extend(netmap.effects());
    }
    Ok((current, effects))
}

pub(crate) fn inventory(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
) -> Result<NodeInventoryResponse, Error> {
    let now = now()?;
    let user = super::super::authorization::current_session(tx, &token, now)?
        .user
        .id;
    let role:String=tx.query_row("SELECT role FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(Error::NotFound)?;
    let manager = role == "owner" || role == "admin";
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let visible = cfg
        .nodes
        .keys()
        .filter(|node| {
            manager || user_binding(&cfg, &bindings, **node).is_some_and(|b| b.user_id == user)
        })
        .copied()
        .collect();
    inventory_in_transaction(tx, sealer, network, &cfg, &visible, manager)
}

pub(crate) fn settings(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
    node: NodeId,
) -> Result<(Revision, meshway_configuration::NodeSettingsView), Error> {
    let user = crate::store::authorization::current_session(tx, &token, now()?)?
        .user
        .id;
    let (member, role) = crate::store::authorization::membership(tx, network, user)?;
    let (revision, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let manager = matches!(role, MembershipRole::Owner | MembershipRole::Admin);
    if !manager
        && !user_binding(&cfg, &bindings, node)
            .is_some_and(|b| b.user_id == user && b.membership_id == member)
    {
        return Err(Error::NotFound);
    }
    Ok((
        revision,
        cfg.nodes.get(&node).ok_or(Error::NotFound)?.into(),
    ))
}

/// Same public inventory fragment for GET and SSE. The caller supplies a scope
/// already authorized inside this transaction; this function grants no access.
pub(crate) fn inventory_in_transaction(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    cfg: &meshway_configuration::NetworkConfiguration,
    visible: &std::collections::BTreeSet<NodeId>,
    manager: bool,
) -> Result<NodeInventoryResponse, Error> {
    let mut items = Vec::new();
    let mut references = BTreeMap::new();
    for (node, settings) in &cfg.nodes {
        if !visible.contains(node) {
            continue;
        }
        // Management inventory is not the execution-eligible inventory. A
        // revoked key must stay visible until its Node is explicitly deleted.
        let (control, control_key_revoked_at_ms, control_key_created_at_ms) =
            latest_key(tx, network, *node, "control", KeyAlgorithm::Ed25519)?;
        let (dataplane, dataplane_key_revoked_at_ms, dataplane_key_created_at_ms) =
            latest_key(tx, network, *node, "dataplane", KeyAlgorithm::X25519)?;
        let registered: i64 = tx.query_row(
            "SELECT registered_at FROM nodes WHERE network_id=?1 AND id=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )?;
        items.push(NodeInventoryView {
            desired_execution: desired_execution(tx, sealer, network, *node, &mut references)?,
            id: *node,
            network_id: network,
            registered_at_ms: registered as u64,
            hostname: settings.hostname.clone(),
            enabled: settings.enabled,
            identity: settings.identity.clone(),
            addresses: settings.addresses.clone(),
            keys: NodeKeyMetadata { control, dataplane },
            control_key_revoked_at_ms,
            dataplane_key_revoked_at_ms,
            can_delete: manager,
            can_revoke_control_key: manager && control_key_revoked_at_ms.is_none(),
            can_revoke_dataplane_key: manager && dataplane_key_revoked_at_ms.is_none(),
            control_key_created_at_ms,
            dataplane_key_created_at_ms,
        });
    }
    Ok(NodeInventoryResponse { items })
}

/// Decrypt each shared publication once per inventory read; retain only public refs.
fn desired_execution(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
    references: &mut BTreeMap<PublicationId, BTreeMap<NodeId, ConfigurationRef>>,
) -> Result<Option<ConfigurationRef>, Error> {
    let publication: Option<Vec<u8>> = tx.query_row(
        "SELECT p.id FROM node_execution e JOIN execution_publications p ON p.network_id=e.network_id AND p.id=e.target_publication_id WHERE e.network_id=?1 AND e.node_id=?2 AND p.withdrawn_at IS NULL",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice()], |row| row.get(0),
    ).optional()?;
    let Some(publication) = publication else {
        return Ok(None);
    };
    let publication: PublicationId = id(publication)?;
    if let std::collections::btree_map::Entry::Vacant(entry) = references.entry(publication) {
        let payload: Vec<u8> = tx.query_row(
            "SELECT payload FROM execution_publications WHERE network_id=?1 AND id=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                publication.as_uuid().as_bytes().as_slice()
            ],
            |row| row.get(0),
        )?;
        let Some(compiled) =
            super::cache::read(tx, sealer, network, publication, &payload)?.current()
        else {
            super::cache::retire(tx, network, publication, now()?)?;
            return Ok(None);
        };
        entry.insert(
            compiled
                .nodes
                .into_iter()
                .map(|(node, target)| {
                    (
                        node,
                        ConfigurationRef {
                            revision: compiled.revision,
                            execution_publication: publication,
                            digest: target.digest,
                        },
                    )
                })
                .collect(),
        );
    }
    references[&publication]
        .get(&node)
        .cloned()
        .map(Some)
        .ok_or(Error::decode("execution_publications.payload"))
}
fn latest_key(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    purpose: &str,
    algorithm: KeyAlgorithm,
) -> Result<(BoundNodeKey, Option<u64>, u64), Error> {
    let (key, public, generation, revoked, created): (Vec<u8>, Vec<u8>, i64, Option<i64>, i64) = tx.query_row(
        "SELECT id,public_key,generation,revoked_at,created_at FROM node_keys WHERE network_id=?1 AND node_id=?2 AND purpose=?3 ORDER BY generation DESC LIMIT 1",
        params![network.as_uuid().as_bytes().as_slice(), node.as_uuid().as_bytes().as_slice(), purpose],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))?;
    Ok((
        BoundNodeKey {
            id: id(key)?,
            generation: Version::new(
                generation
                    .try_into()
                    .map_err(|_| Error::decode("node_keys.generation"))?,
            )
            .map_err(|_| Error::decode("node_keys.generation"))?,
            public_key: NodePublicKey {
                algorithm,
                bytes: Digest::from_bytes(
                    public
                        .try_into()
                        .map_err(|_| Error::decode("node_keys.public_key"))?,
                ),
            },
        },
        revoked
            .map(u64::try_from)
            .transpose()
            .map_err(|_| Error::decode("node_keys.revoked_at"))?,
        created
            .try_into()
            .map_err(|_| Error::decode("node_keys.created_at"))?,
    ))
}
pub(crate) fn progress(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    network: NetworkId,
    publication: PublicationId,
) -> Result<PublicationProgress, Error> {
    let user = super::super::authorization::current_session(tx, &token, now()?)?
        .user
        .id;
    let manager:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships WHERE network_id=?1 AND user_id=?2 AND role IN ('owner','admin') AND revoked_at IS NULL)",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if !manager {
        return Err(Error::PermissionDenied);
    }
    let (payload,sequence,mut withdrawn):(Vec<u8>,i64,Option<i64>)=tx.query_row("SELECT payload,sequence,withdrawn_at FROM execution_publications WHERE network_id=?1 AND id=?2",params![network.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(Error::NotFound)?;
    let stored = super::cache::read(tx, sealer, network, publication, &payload)?;
    if matches!(&stored, super::cache::Stored::Obsolete { .. }) {
        let now = now()?;
        super::cache::retire(tx, network, publication, now)?;
        withdrawn = withdrawn.or(Some(now));
    }
    let (revision, references) = stored.references();
    let mut nodes = BTreeMap::new();
    for (node, reference) in references {
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND target_publication_id=?3)",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
        let phase = |phase: &str| -> Result<bool, Error> {
            Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_receipts r JOIN node_execution e ON e.network_id=r.network_id AND e.node_id=r.node_id AND e.instance_id=r.instance_id AND e.generation=r.generation WHERE r.network_id=?1 AND r.node_id=?2 AND r.publication_id=?3 AND r.phase=?4)",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice(),phase],|r|r.get(0))?)
        };
        let freshness = crate::state::progress::freshness(
            tx,
            network,
            node,
            &reference,
            None,
            boot,
            chrono::Utc::now(),
        )?;
        let last_received_at_ms:Option<i64>=tx.query_row("SELECT max(received_at) FROM execution_receipts WHERE network_id=?1 AND node_id=?2 AND publication_id=?3",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
        // Arrival order is not application order. A new attempt (even only
        // saved so far) supersedes an older result; late evidence stays stored.
        let result: Option<String> = tx
            .query_row(
                "SELECT r.phase FROM execution_receipts r JOIN node_execution e
             ON e.network_id=r.network_id AND e.node_id=r.node_id
             AND e.instance_id=r.instance_id AND e.generation=r.generation
             WHERE r.network_id=?1 AND r.node_id=?2 AND r.publication_id=?3
             ORDER BY r.attempt DESC,r.sequence DESC LIMIT 1",
                params![
                    network.as_uuid().as_bytes().as_slice(),
                    node.as_uuid().as_bytes().as_slice(),
                    publication.as_uuid().as_bytes().as_slice()
                ],
                |r| r.get(0),
            )
            .optional()?;
        nodes.insert(
            node,
            NodePublicationProgress {
                target: reference,
                current,
                saved: phase("saved")?,
                applied: freshness == ReceiptFreshness::Current
                    && result.as_deref() == Some("applied"),
                failed: result.as_deref() == Some("failed"),
                freshness,
                last_received_at_ms: last_received_at_ms.map(|v| v as u64),
            },
        );
    }
    Ok(PublicationProgress {
        publication,
        revision,
        sequence: Sequence::new(
            sequence
                .try_into()
                .map_err(|_| Error::decode("execution_publications.sequence"))?,
        )
        .map_err(|_| Error::decode("execution_publications.sequence"))?,
        withdrawn_at_ms: withdrawn.map(|v| v as u64),
        nodes,
    })
}
