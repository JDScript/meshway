use super::*;
use meshway_configuration::{
    compile::{self, CompilationInput},
    execution::*,
};
use meshway_protocol::execution::*;
use rusqlite::{Connection, TransactionBehavior};
use std::collections::BTreeMap;

pub(crate) fn select_profile(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: RoutingSelectionRequest,
) -> Result<RoutingSelectionResponse, NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request.instance, now)?;
    let (_, cfg, bindings) = current_configuration(&tx, sealer, p.network)?;
    if !compile::profile_allowed(&cfg, p.node, &request.profile)? {
        return Err(NodeError::Forbidden);
    }
    let digest = compile::digest(&(
        &cfg.nodes[&p.node].identity,
        user_binding(&cfg, &bindings, p.node),
    ))?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT version FROM routing_selections WHERE network_id=?1 AND node_id=?2",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                p.node.as_uuid().as_bytes().as_slice()
            ],
            |r| r.get(0),
        )
        .optional()?;
    if current != request.expected_version.map(|v| v.get() as i64) {
        return Err(NodeError::Conflict);
    }
    let version = Version::new(current.unwrap_or(0) as u64 + 1).map_err(|_| NodeError::Conflict)?;
    tx.execute("INSERT INTO routing_selections(network_id,node_id,identity_digest,profile,version,selected_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(network_id,node_id) DO UPDATE SET identity_digest=excluded.identity_digest,profile=excluded.profile,version=excluded.version,selected_at=excluded.selected_at",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),request.profile,version.get() as i64,now])?;
    tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    let publication = publish(&tx, sealer, p.network, "refresh", now)?.publication;
    tx.commit()?;
    Ok(RoutingSelectionResponse {
        version,
        profile: request.profile,
        publication,
    })
}

pub(crate) fn trusted_inventory(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
) -> Result<BTreeMap<NodeId, TrustedNode>, NodeError> {
    let (_, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let mut out = BTreeMap::new();
    let mut stmt=tx.prepare("SELECT n.id,k.id,k.public_key,k.generation,d.id,d.public_key,d.generation,e.capabilities FROM nodes n JOIN node_keys k ON k.network_id=n.network_id AND k.node_id=n.id AND k.purpose='control' AND k.retired_at IS NULL AND k.revoked_at IS NULL JOIN node_keys d ON d.network_id=n.network_id AND d.node_id=n.id AND d.purpose='dataplane' AND d.retired_at IS NULL AND d.revoked_at IS NULL LEFT JOIN node_execution e ON e.network_id=n.network_id AND e.node_id=n.id WHERE n.network_id=?1 AND n.credentials_revoked_at IS NULL")?;
    let mut rows = stmt.query([network.as_uuid().as_bytes().as_slice()])?;
    while let Some(r) = rows.next()? {
        let node = id(r.get(0)?)?;
        match identity_eligible(tx, &cfg, &bindings, network, node) {
            Ok(()) => {}
            Err(NodeError::Unauthorized) => continue,
            Err(error) => return Err(error),
        }
        let key = |offset| -> Result<BoundNodeKey, NodeError> {
            Ok(BoundNodeKey {
                id: id(r.get(offset)?)?,
                generation: Version::new(
                    r.get::<_, i64>(offset + 2)?
                        .try_into()
                        .map_err(|_| NodeError::Storage)?,
                )
                .map_err(|_| NodeError::Storage)?,
                public_key: NodePublicKey {
                    algorithm: if offset == 1 {
                        KeyAlgorithm::Ed25519
                    } else {
                        KeyAlgorithm::X25519
                    },
                    bytes: Digest::from_bytes(
                        r.get::<_, Vec<u8>>(offset + 1)?
                            .try_into()
                            .map_err(|_| NodeError::Storage)?,
                    ),
                },
            })
        };
        let capabilities = r
            .get::<_, Option<Vec<u8>>>(7)?
            .map(|v| json(&v))
            .transpose()?
            .unwrap_or_default();
        out.insert(
            node,
            TrustedNode {
                control_key: key(1)?,
                dataplane_key: key(4)?,
                capabilities,
            },
        );
    }
    Ok(out)
}
pub(crate) fn publish(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    cause: &str,
    now: i64,
) -> Result<CompiledPublication, NodeError> {
    let (revision, cfg, bindings) = current_configuration(tx, sealer, network)?;
    let mut selected_profiles = BTreeMap::new();
    let mut statement = tx.prepare(
        "SELECT node_id,identity_digest,profile FROM routing_selections WHERE network_id=?1",
    )?;
    let mut rows = statement.query([network.as_uuid().as_bytes().as_slice()])?;
    while let Some(row) = rows.next()? {
        let node: NodeId = id(row.get(0)?)?;
        let stored: Vec<u8> = row.get(1)?;
        if let Some(settings) = cfg.nodes.get(&node) {
            let digest =
                compile::digest(&(&settings.identity, user_binding(&cfg, &bindings, node)))?;
            if stored == digest.as_bytes() {
                selected_profiles.insert(node, row.get::<_, String>(2)?);
            }
        }
    }
    drop(rows);
    drop(statement);
    let inventory = trusted_inventory(tx, sealer, network)?;
    let mut stmt = tx.prepare(
        "SELECT payload FROM execution_demands WHERE network_id=?1 AND expires_at>?2 ORDER BY id",
    )?;
    let demands = stmt
        .query_map(params![network.as_uuid().as_bytes().as_slice(), now], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|v| {
            let d: Demand = json(&v?)?;
            Ok((d.id, d))
        })
        .collect::<Result<BTreeMap<_, _>, NodeError>>()?;
    drop(stmt);
    let publication = PublicationId::new(uuid()?).map_err(|_| NodeError::Storage)?;
    let compiled = compile::compile(CompilationInput {
        network,
        revision,
        publication,
        configuration: &cfg,
        inventory: &inventory,
        demands: &demands,
        dns: &[],
        rule_sets: &BTreeMap::new(),
        selected_profiles: &selected_profiles,
        now_ms: now as u64,
    })?;
    let mut stmt=tx.prepare("SELECT DISTINCT target_publication_id FROM node_execution WHERE network_id=?1 AND target_publication_id IS NOT NULL")?;
    let supersedes = stmt
        .query_map([network.as_uuid().as_bytes().as_slice()], |r| {
            r.get::<_, Vec<u8>>(0)
        })?
        .map(|v| id::<PublicationId>(v?))
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    tx.execute(
        "UPDATE networks SET publication_sequence=publication_sequence+1 WHERE id=?1",
        [network.as_uuid().as_bytes().as_slice()],
    )?;
    let sequence: i64 = tx.query_row(
        "SELECT publication_sequence FROM networks WHERE id=?1",
        [network.as_uuid().as_bytes().as_slice()],
        |r| r.get(0),
    )?;
    let payload = zeroize::Zeroizing::new(bytes(&compiled)?);
    let sealed = sealer
        .seal_execution(network, publication, &payload)
        .map_err(|_| NodeError::Storage)?;
    tx.execute("INSERT INTO execution_publications(id,network_id,sequence,revision,cause,supersedes,dependency_digest,payload,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![publication.as_uuid().as_bytes().as_slice(),network.as_uuid().as_bytes().as_slice(),sequence,revision.get() as i64,cause,bytes(&supersedes)?,compiled.dependency_digest.as_bytes().as_slice(),sealed.as_bytes(),now])?;
    // All concurrent demands were included; superseding never loses an unrelated grant.
    tx.execute("UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL,saved=NULL WHERE network_id=?1",[network.as_uuid().as_bytes().as_slice()])?;
    for node in compiled.nodes.keys() {
        tx.execute(
            "UPDATE node_execution SET target_publication_id=?1 WHERE network_id=?2 AND node_id=?3",
            params![
                publication.as_uuid().as_bytes().as_slice(),
                network.as_uuid().as_bytes().as_slice(),
                node.as_uuid().as_bytes().as_slice()
            ],
        )?;
    }
    for old in supersedes {
        tx.execute("UPDATE execution_publications SET withdrawn_at=COALESCE(withdrawn_at,?1) WHERE network_id=?2 AND id=?3",params![now,network.as_uuid().as_bytes().as_slice(),old.as_uuid().as_bytes().as_slice()])?;
    }
    Ok(compiled)
}
pub(crate) fn target(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
) -> Result<Option<(PublicationId, CompiledNode)>, NodeError> {
    target_for_node(tx, sealer, p.network, p.node)
}
/// SQL facts only. The caller must authorize its viewer in this transaction.
pub(crate) fn target_for_node(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    node: NodeId,
) -> Result<Option<(PublicationId, CompiledNode)>, NodeError> {
    let row:Option<(Vec<u8>,Vec<u8>)>=tx.query_row("SELECT x.id,x.payload FROM node_execution e JOIN execution_publications x ON x.network_id=e.network_id AND x.id=e.target_publication_id WHERE e.network_id=?1 AND e.node_id=?2 AND x.withdrawn_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(publication, payload)| {
        let publication = id(publication)?;
        let plain = sealer
            .open_execution(network, publication, &payload)
            .map_err(|_| NodeError::Storage)?;
        let mut compiled: CompiledPublication = json(plain.as_bytes())?;
        let node = compiled.nodes.remove(&node).ok_or(NodeError::Storage)?;
        Ok((publication, node))
    })
    .transpose()
}
pub(crate) use target as current_target;
pub(super) fn historical_target(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
    reference: &ConfigurationRef,
) -> Result<Option<CompiledNode>, NodeError> {
    let payload:Option<Vec<u8>>=tx.query_row("SELECT payload FROM execution_publications WHERE network_id=?1 AND id=?2 AND revision=?3",params![p.network.as_uuid().as_bytes().as_slice(),reference.execution_publication.as_uuid().as_bytes().as_slice(),reference.revision.get() as i64],|r|r.get(0)).optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    let mut compiled: CompiledPublication = json(
        sealer
            .open_execution(p.network, reference.execution_publication, &payload)
            .map_err(|_| NodeError::Storage)?
            .as_bytes(),
    )?;
    Ok(compiled
        .nodes
        .remove(&p.node)
        .filter(|n| n.digest == reference.digest))
}
pub(crate) fn check_publication(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    publication: PublicationId,
    now: i64,
) -> Result<(), NodeError> {
    let payload:Vec<u8>=tx.query_row("SELECT x.payload FROM execution_publications x JOIN networks n ON n.id=x.network_id AND n.current_revision=x.revision WHERE x.network_id=?1 AND x.id=?2 AND x.withdrawn_at IS NULL",params![network.as_uuid().as_bytes().as_slice(),publication.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?.ok_or(NodeError::Conflict)?;
    let plain = sealer
        .open_execution(network, publication, &payload)
        .map_err(|_| NodeError::Storage)?;
    let compiled: CompiledPublication = json(plain.as_bytes())?;
    let current = trusted_inventory(tx, sealer, network)?;
    for (id, node) in compiled.nodes {
        if current.get(&id) != Some(&node.configuration.self_keys) {
            return Err(NodeError::Conflict);
        }
        if node
            .configuration
            .permits
            .values()
            .any(|p| p.expires_at_ms <= now as u64)
        {
            return Err(NodeError::Conflict);
        }
    }
    Ok(())
}
pub(super) fn refresh_expired_demands(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    now: i64,
) -> Result<(), NodeError> {
    if tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND expires_at<=?2",
        params![network.as_uuid().as_bytes().as_slice(), now],
    )? > 0
    {
        publish(tx, sealer, network, "refresh", now)?;
    }
    Ok(())
}
pub(crate) fn revoke_execution(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
    now: i64,
) -> Result<(), NodeError> {
    // Withdrawal of the network's current targets is conservative but immediate;
    // a new complete compilation must precede any further execution lease.
    tx.execute("UPDATE execution_publications SET withdrawn_at=COALESCE(withdrawn_at,?1) WHERE network_id=?2",params![now,network.as_uuid().as_bytes().as_slice()])?;
    tx.execute("UPDATE node_execution SET target_publication_id=NULL,execution_expires_at=NULL,saved=NULL WHERE network_id=?1",[network.as_uuid().as_bytes().as_slice()])?;
    tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2",
        params![
            network.as_uuid().as_bytes().as_slice(),
            node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok(())
}
/// Config owner invokes within its source commit transaction after writing the
/// authoritative target. Removed nodes are permanent revocations, including JSONC deletion.
pub(crate) fn apply_configuration_effects(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    previous: &NetworkConfiguration,
    target: &NetworkConfiguration,
    now: i64,
) -> Result<PublicationId, NodeError> {
    let (revision, committed, _) = current_configuration(tx, sealer, network)?;
    if &committed != target {
        return Err(NodeError::Conflict);
    }
    for (node, old) in &previous.nodes {
        if target
            .nodes
            .get(node)
            .is_some_and(|new| new.identity != old.identity)
        {
            tx.execute(
                "DELETE FROM node_sessions WHERE network_id=?1 AND node_id=?2",
                params![
                    network.as_uuid().as_bytes().as_slice(),
                    node.as_uuid().as_bytes().as_slice()
                ],
            )?;
            tx.execute(
                "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2",
                params![
                    network.as_uuid().as_bytes().as_slice(),
                    node.as_uuid().as_bytes().as_slice()
                ],
            )?;
            tx.execute("UPDATE node_execution SET instance_expires_at=?1,execution_expires_at=NULL,saved=NULL WHERE network_id=?2 AND node_id=?3",params![now,network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()])?;
        }
    }
    for (node, old) in &previous.nodes {
        let addresses = [
            old.addresses.ipv4.map(std::net::IpAddr::V4),
            old.addresses.ipv6.map(std::net::IpAddr::V6),
        ];
        let new = target
            .nodes
            .get(node)
            .map(|n| {
                [
                    n.addresses.ipv4.map(std::net::IpAddr::V4),
                    n.addresses.ipv6.map(std::net::IpAddr::V6),
                ]
            })
            .unwrap_or([None, None]);
        for address in addresses
            .into_iter()
            .flatten()
            .filter(|a| !new.contains(&Some(*a)))
        {
            tx.execute("INSERT INTO address_retirements(network_id,address,previous_node_id,removed_revision,release_after) VALUES(?1,?2,?3,?4,NULL)",params![network.as_uuid().as_bytes().as_slice(),address.to_string(),node.as_uuid().as_bytes().as_slice(),revision.get() as i64])?;
        }
    }
    for node in previous
        .nodes
        .keys()
        .filter(|id| !target.nodes.contains_key(id))
    {
        tx.execute("UPDATE nodes SET credentials_revoked_at=COALESCE(credentials_revoked_at,?1) WHERE network_id=?2 AND id=?3",params![now,network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()])?;
        revoke_execution(tx, network, *node, now)?;
    }
    Ok(publish(tx, sealer, network, "configuration", now)?.publication)
}
pub(crate) fn demand(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: DemandRequest,
) -> Result<DemandResponse, NodeError> {
    if !(1..=300).contains(&request.duration_seconds) {
        return Err(NodeError::Invalid);
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request.instance, now)?;
    let digest = crate::execution::proof::request_digest(&request)?;
    let previous:Option<Vec<u8>>=tx.query_row("SELECT request_digest FROM execution_demands WHERE network_id=?1 AND node_id=?2 AND id=?3",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.request_id.as_uuid().as_bytes().as_slice()],|r|r.get(0)).optional()?;
    if previous.as_deref().is_some_and(|v| v != digest.as_bytes()) {
        return Err(NodeError::Conflict);
    }
    if previous.is_none() {
        let d = Demand {
            id: request.request_id,
            source: p.node,
            target: request.target,
            protocol: request.protocol,
            port: request.port,
            profile: request.profile,
            expires_at_ms: (now + i64::from(request.duration_seconds) * 1000) as u64,
        };
        tx.execute("INSERT INTO execution_demands(network_id,node_id,id,instance_id,request_digest,payload,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),d.id.as_uuid().as_bytes().as_slice(),request.instance.instance_id.as_uuid().as_bytes().as_slice(),digest.as_bytes().as_slice(),bytes(&d)?,d.expires_at_ms as i64])?;
    }
    let compiled = publish(&tx, sealer, p.network, "demand", now)?;
    let decision = compiled
        .nodes
        .get(&p.node)
        .and_then(|n| n.configuration.decisions.get(&request.request_id))
        .cloned()
        .ok_or(NodeError::Conflict)?;
    grant_lease(&tx, sealer, &p, now)?;
    tx.commit()?;
    Ok(DemandResponse {
        publication: compiled.publication,
        decision,
    })
}
pub(crate) fn cancel_demand(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: CancelDemandRequest,
) -> Result<(), NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request.instance, now)?;
    if tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2 AND id=?3",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice(),
            request.request_id.as_uuid().as_bytes().as_slice()
        ],
    )? > 0
    {
        publish(&tx, sealer, p.network, "demand", now)?;
    }
    tx.commit()?;
    Ok(())
}
