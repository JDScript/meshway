use super::{current_configuration, id, now, user_binding};
use crate::effect::Effect;
use crate::error::Error;
use crate::secrets::Sealer;
use meshway_model::{
    BoundNodeKey, Digest, KeyAlgorithm, MembershipRole, NetworkId, NodeId, NodePublicKey, Revision,
    Version,
};
use meshway_protocol::node::*;
use rusqlite::{OptionalExtension, Transaction, params};
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
        super::retire_node_sessions(tx, network, node, now)?;
        let netmap = crate::store::netmap::recompile(
            tx,
            sealer,
            network,
            now,
            crate::store::netmap::Trigger::Node,
        )?;
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
    inventory_in_transaction(tx, network, &cfg, &bindings, &visible, manager)
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
    network: NetworkId,
    cfg: &meshway_configuration::NetworkConfiguration,
    bindings: &crate::configuration::ConfigurationBindings,
    visible: &std::collections::BTreeSet<NodeId>,
    manager: bool,
) -> Result<NodeInventoryResponse, Error> {
    let now = now()?;
    let subjects = super::subjects(tx, network, bindings)?;
    let mut items = Vec::new();
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
        let record = crate::netmap::records::Records::shared().get(network, *node);
        let selection = super::selection::state(tx, network, cfg, bindings, &subjects, *node)?;
        items.push(NodeInventoryView {
            routing: meshway_protocol::NodeRoutingSummary {
                can_select: super::selection::can_select(&selection),
                requested_profile: selection.requested_profile,
                applied_profile: selection.applied_profile,
                locked: selection.locked,
            },
            netmap: crate::store::netmap::current(tx, network, *node)?.map(|c| c.current),
            netmap_unavailable: crate::store::netmap::unavailability(tx, network, *node)?,
            lease: record.lease,
            last_sync_at_ms: record.synced_at_ms,
            server_key: record
                .pinned_key
                .map(|key_generation| {
                    crate::store::netmap::pinned_status(tx, now, key_generation).map(|status| {
                        meshway_protocol::PinnedServerKeyView {
                            key_generation,
                            status,
                        }
                    })
                })
                .transpose()?,
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
