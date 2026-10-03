use super::*;
use meshway_protocol::execution::*;
use rusqlite::{Connection, TransactionBehavior};

pub(crate) const INSTANCE_TTL: i64 = 120_000;
pub(crate) const EXECUTION_TTL: i64 = 60_000;
pub(crate) use lease as instance_lease;
struct PreviousInstance {
    instance: Option<Vec<u8>>,
    generation: i64,
    boot: Option<Vec<u8>>,
    expires: Option<i64>,
    capabilities: Vec<u8>,
}

/// A real current instance is required before State/observations can be accepted.
pub(crate) fn current_instance(
    tx: &Transaction<'_>,
    boot: InstanceId,
    p: &NodePrincipal,
    request: &InstanceRequest,
    now: i64,
) -> Result<(), NodeError> {
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND instance_id=?3 AND generation=?4 AND boot_id=?5 AND instance_expires_at>?6)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),request.generation.get() as i64,boot.as_uuid().as_bytes().as_slice(),now],|r|r.get(0))?;
    if valid {
        Ok(())
    } else {
        Err(NodeError::Conflict)
    }
}
pub(crate) fn acquire(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: AcquireInstanceRequest,
) -> Result<InstanceLease, NodeError> {
    if request.capabilities.len() > 32
        || request.capabilities.iter().any(|(k, v)| {
            k.len() > 64
                || !k.starts_with("transport:")
                || v.streams.len() > 8
                || v.streams.iter().any(|s| s.len() > 32)
        })
    {
        return Err(NodeError::Invalid);
    }
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    let previous=tx.query_row("SELECT instance_id,generation,boot_id,instance_expires_at,capabilities FROM node_execution WHERE network_id=?1 AND node_id=?2",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice()],|r|Ok(PreviousInstance{instance:r.get(0)?,generation:r.get(1)?,boot:r.get(2)?,expires:r.get(3)?,capabilities:r.get(4)?})).optional()?;
    let caps = bytes(&request.capabilities)?;
    let generation = if let Some(PreviousInstance {
        instance,
        generation,
        boot: old_boot,
        expires,
        capabilities: old_caps,
    }) = &previous
    {
        if instance.as_deref() == Some(request.instance_id.as_uuid().as_bytes().as_slice())
            && old_boot.as_deref() == Some(boot.as_uuid().as_bytes().as_slice())
            && expires.is_some_and(|t| t > now)
        {
            if old_caps != &caps {
                return Err(NodeError::Conflict);
            }
            let lease = lease(&tx, sealer, &p)?;
            tx.commit()?;
            return Ok(lease);
        }
        generation + 1
    } else {
        1
    };
    Version::new(generation.try_into().map_err(|_| NodeError::Conflict)?)
        .map_err(|_| NodeError::Conflict)?;
    let claimed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_instance_claims WHERE network_id=?1 AND node_id=?2 AND instance_id=?3)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if claimed {
        return Err(NodeError::Conflict);
    }
    tx.execute("INSERT INTO node_instance_claims(network_id,node_id,instance_id,generation) VALUES(?1,?2,?3,?4)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),generation])?;
    // Replacing a process fences its demands as well as its ACK/report authority.
    tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    tx.execute("INSERT INTO node_execution(network_id,node_id,instance_id,generation,boot_id,instance_expires_at,capabilities) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(network_id,node_id) DO UPDATE SET instance_id=excluded.instance_id,generation=excluded.generation,boot_id=excluded.boot_id,instance_expires_at=excluded.instance_expires_at,execution_expires_at=NULL,saved=NULL,capabilities=excluded.capabilities",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),generation,boot.as_uuid().as_bytes().as_slice(),now+INSTANCE_TTL,caps])?;
    publish(&tx, sealer, p.network, "refresh", now)?;
    grant_lease(&tx, sealer, &p, now)?;
    let result = lease(&tx, sealer, &p)?;
    tx.commit()?;
    Ok(result)
}
pub(crate) fn renew(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: InstanceRequest,
) -> Result<InstanceLease, NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request, now)?;
    refresh_expired_demands(&tx, sealer, p.network, now)?;
    tx.execute(
        "UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3",
        params![
            now + INSTANCE_TTL,
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    grant_lease(&tx, sealer, &p, now)?;
    let result = lease(&tx, sealer, &p)?;
    tx.commit()?;
    Ok(result)
}
pub(crate) fn release(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: InstanceRequest,
) -> Result<(), NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request, now)?;
    tx.execute("UPDATE node_execution SET instance_expires_at=?1,execution_expires_at=NULL,saved=NULL WHERE network_id=?2 AND node_id=?3",params![now,p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice()])?;
    tx.execute(
        "DELETE FROM execution_demands WHERE network_id=?1 AND node_id=?2",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    publish(&tx, sealer, p.network, "refresh", now)?;
    tx.commit()?;
    Ok(())
}
pub(super) fn grant_lease(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
    now: i64,
) -> Result<(), NodeError> {
    let (revision, cfg, _) = current_configuration(tx, sealer, p.network)?;
    let target = target(tx, sealer, p)?;
    let mut address_blocked = false;
    if let Some((_, node)) = &target {
        for addresses in std::iter::once(&node.configuration.addresses)
            .chain(node.configuration.peers.values().map(|p| &p.addresses))
        {
            for address in [
                addresses.ipv4.map(std::net::IpAddr::V4),
                addresses.ipv6.map(std::net::IpAddr::V6),
            ]
            .into_iter()
            .flatten()
            {
                address_blocked |= tx.query_row("SELECT EXISTS(SELECT 1 FROM address_retirements WHERE network_id=?1 AND address=?2 AND (release_after IS NULL OR release_after>?3))",params![p.network.as_uuid().as_bytes().as_slice(),address.to_string(),now],|r|r.get::<_,bool>(0))?;
            }
        }
    }
    let expires = if !address_blocked
        && cfg.nodes.get(&p.node).is_some_and(|n| n.enabled)
        && target
            .as_ref()
            .is_some_and(|(_, c)| c.configuration.revision == revision)
    {
        // Publication eligibility checks every peer key/current identity, not only this node.
        if let Some((publication, _)) = &target {
            check_publication(tx, sealer, p.network, *publication, now)?;
        }
        Some(now + EXECUTION_TTL)
    } else {
        None
    };
    tx.execute(
        "UPDATE node_execution SET execution_expires_at=?1 WHERE network_id=?2 AND node_id=?3",
        params![
            expires,
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok(())
}
pub(crate) fn lease(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    p: &NodePrincipal,
) -> Result<InstanceLease, NodeError> {
    let (instance,generation,expires,execution):(Vec<u8>,i64,i64,Option<i64>)=tx.query_row("SELECT instance_id,generation,instance_expires_at,execution_expires_at FROM node_execution WHERE network_id=?1 AND node_id=?2",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let target = target(tx, sealer, p)?.map(|(publication, c)| ConfigurationRef {
        revision: c.configuration.revision,
        execution_publication: publication,
        digest: c.digest,
    });
    Ok(InstanceLease {
        instance_id: id(instance)?,
        generation: Version::new(generation.try_into().map_err(|_| NodeError::Storage)?)
            .map_err(|_| NodeError::Storage)?,
        expires_at_ms: expires.try_into().map_err(|_| NodeError::Storage)?,
        execution_expires_at_ms: execution
            .map(u64::try_from)
            .transpose()
            .map_err(|_| NodeError::Storage)?,
        target,
    })
}
pub(crate) fn configuration(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: ConfigurationRequest,
) -> Result<ConfigurationResponse, NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request.instance, now)?;
    refresh_expired_demands(&tx, sealer, p.network, now)?;
    let lease = lease(&tx, sealer, &p)?;
    let full = if request.base == lease.target {
        None
    } else {
        target(&tx, sealer, &p)?.map(|(_, c)| c)
    };
    let mut delta = None;
    let mut full = full;
    if let (Some(base_ref), Some(target)) = (&request.base, &full)
        && let Some(base) = historical_target(&tx, sealer, &p, base_ref)?
    {
        delta = Some(meshway_configuration::execution::configuration_delta(
            &base, target,
        )?);
        full = None;
    }
    tx.commit()?;
    Ok(ConfigurationResponse { lease, full, delta })
}
pub(crate) fn receipt(
    connection: &mut Connection,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: ReceiptRequest,
) -> Result<(), NodeError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = now()?;
    let p = authorize_node(&tx, sealer, boot, token, now)?;
    current_instance(&tx, boot, &p, &request.instance, now)?;
    let lease = lease(&tx, sealer, &p)?;
    if lease.target.as_ref() != Some(&request.target)
        || request.reported_at_ms > now as u64 + 5000
        || request.reported_at_ms.saturating_add(300_000) < now as u64
        || (request.phase == ReceiptPhase::Failed) != request.failure.is_some()
    {
        return Err(NodeError::Conflict);
    }
    if request.phase == ReceiptPhase::Applied
        && lease
            .execution_expires_at_ms
            .is_none_or(|e| e <= now as u64)
    {
        return Err(NodeError::Conflict);
    }
    check_publication(
        &tx,
        sealer,
        p.network,
        request.target.execution_publication,
        now,
    )?;
    let phase = match request.phase {
        ReceiptPhase::Saved => "saved",
        ReceiptPhase::Applied => "applied",
        ReceiptPhase::Failed => "failed",
    };
    if request.phase == ReceiptPhase::Applied {
        let saved:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM execution_receipts WHERE network_id=?1 AND publication_id=?2 AND node_id=?3 AND instance_id=?4 AND phase='saved')",params![p.network.as_uuid().as_bytes().as_slice(),request.target.execution_publication.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance.instance_id.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
        if !saved {
            return Err(NodeError::Conflict);
        }
    }
    let failure = request.failure.map(|f| match f {
        ApplyFailure::Validation => "validation",
        ApplyFailure::Storage => "storage",
        ApplyFailure::Listeners => "listeners",
        ApplyFailure::Underlay => "underlay",
        ApplyFailure::Tun => "tun",
        ApplyFailure::Dns => "dns",
        ApplyFailure::Dataplane => "dataplane",
    });
    let old:Option<(i64,Option<String>)>=tx.query_row("SELECT reported_at,failure FROM execution_receipts WHERE network_id=?1 AND publication_id=?2 AND node_id=?3 AND instance_id=?4 AND phase=?5",params![p.network.as_uuid().as_bytes().as_slice(),request.target.execution_publication.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance.instance_id.as_uuid().as_bytes().as_slice(),phase],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some(old) = old {
        if old != (request.reported_at_ms as i64, failure.map(str::to_owned)) {
            return Err(NodeError::Conflict);
        }
    } else {
        tx.execute("INSERT INTO execution_receipts(network_id,publication_id,node_id,instance_id,generation,digest,phase,reported_at,received_at,failure) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![p.network.as_uuid().as_bytes().as_slice(),request.target.execution_publication.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance.instance_id.as_uuid().as_bytes().as_slice(),request.instance.generation.get() as i64,request.target.digest.as_bytes().as_slice(),phase,request.reported_at_ms as i64,now,failure])?;
    }
    if request.phase == ReceiptPhase::Saved {
        tx.execute(
            "UPDATE node_execution SET saved=?1 WHERE network_id=?2 AND node_id=?3",
            params![
                bytes(&request)?,
                p.network.as_uuid().as_bytes().as_slice(),
                p.node.as_uuid().as_bytes().as_slice()
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}
