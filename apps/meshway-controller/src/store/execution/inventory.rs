use super::*;
use meshway_protocol::{execution::*, node::*};
use rusqlite::Connection;
use std::collections::BTreeMap;
pub(crate) fn revoke_key(
    c: &mut Connection,
    token: [u8; 32],
    network: NetworkId,
    node: NodeId,
    request: RevokeNodeKeyRequest,
) -> Result<(), NodeError> {
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = now()?;
    let user = super::super::authorization::current_session(&tx, &token, now)?
        .user
        .id;
    crate::configuration::authorize_manager(&tx, network, user)
        .map_err(|_| NodeError::Forbidden)?;
    let found:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_keys WHERE network_id=?1 AND node_id=?2 AND id=?3 AND generation=?4)",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),request.key_id.as_uuid().as_bytes().as_slice(),request.expected_generation.get() as i64],|r|r.get(0))?;
    if !found {
        return Err(NodeError::Conflict);
    }
    tx.execute("UPDATE node_keys SET revoked_at=COALESCE(revoked_at,?1) WHERE network_id=?2 AND node_id=?3 AND id=?4",params![now,network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),request.key_id.as_uuid().as_bytes().as_slice()])?;
    revoke_execution(&tx, network, node, now)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn inventory(
    connection: &mut Connection,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
) -> Result<NodeInventoryResponse, NodeError> {
    let tx = connection.transaction()?;
    let now = now()?;
    let user = super::super::authorization::current_session(&tx, &token, now)?
        .user
        .id;
    let role:String=tx.query_row("SELECT role FROM memberships WHERE network_id=?1 AND user_id=?2 AND revoked_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(NodeError::NotFound)?;
    let manager = role == "owner" || role == "admin";
    let (_, cfg, bindings) = current_configuration(&tx, sealer, network)?;
    let trusted = trusted_inventory(&tx, sealer, network)?;
    let mut items = Vec::new();
    for (node, keys) in trusted {
        let own = user_binding(&cfg, &bindings, node).is_some_and(|b| b.user_id == user);
        if !manager && !own {
            continue;
        }
        let settings = &cfg.nodes[&node];
        let registered: i64 = tx.query_row(
            "SELECT registered_at FROM nodes WHERE network_id=?1 AND id=?2",
            params![
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )?;
        items.push(NodeInventoryView {
            id: node,
            network_id: network,
            registered_at_ms: registered as u64,
            hostname: settings.hostname.clone(),
            enabled: settings.enabled,
            identity: settings.identity.clone(),
            addresses: settings.addresses.clone(),
            keys: NodeKeyMetadata {
                control: keys.control_key,
                dataplane: keys.dataplane_key,
            },
            can_delete: manager,
        });
    }
    Ok(NodeInventoryResponse { items })
}
pub(crate) fn progress(
    connection: &mut Connection,
    sealer: &Sealer,
    token: [u8; 32],
    network: NetworkId,
    publication: PublicationId,
) -> Result<PublicationProgress, NodeError> {
    let tx = connection.transaction()?;
    let user = super::super::authorization::current_session(&tx, &token, now()?)?
        .user
        .id;
    let manager:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships WHERE network_id=?1 AND user_id=?2 AND role IN ('owner','admin') AND revoked_at IS NULL)",params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if !manager {
        return Err(NodeError::Forbidden);
    }
    let (payload,sequence,withdrawn):(Vec<u8>,i64,Option<i64>)=tx.query_row("SELECT payload,sequence,withdrawn_at FROM execution_publications WHERE network_id=?1 AND id=?2",params![network.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.ok_or(NodeError::NotFound)?;
    let compiled: meshway_configuration::execution::CompiledPublication = json(
        sealer
            .open_execution(network, publication, &payload)
            .map_err(|_| NodeError::Storage)?
            .as_bytes(),
    )?;
    let mut nodes = BTreeMap::new();
    for (node, c) in compiled.nodes {
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND target_publication_id=?3)",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
        let phase = |phase: &str| -> Result<bool, NodeError> {
            Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_receipts r JOIN node_execution e ON e.network_id=r.network_id AND e.node_id=r.node_id AND e.instance_id=r.instance_id AND e.generation=r.generation WHERE r.network_id=?1 AND r.node_id=?2 AND r.publication_id=?3 AND r.phase=?4)",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice(),phase],|r|r.get(0))?)
        };
        nodes.insert(
            node,
            NodePublicationProgress {
                target: ConfigurationRef {
                    revision: compiled.revision,
                    execution_publication: publication,
                    digest: c.digest,
                },
                current,
                saved: phase("saved")?,
                applied: phase("applied")?,
                failed: phase("failed")?,
            },
        );
    }
    Ok(PublicationProgress {
        publication,
        revision: compiled.revision,
        sequence: Sequence::new(sequence.try_into().map_err(|_| NodeError::Storage)?)
            .map_err(|_| NodeError::Storage)?,
        withdrawn_at_ms: withdrawn.map(|v| v as u64),
        nodes,
    })
}
