//! Runtime instances and netmap receipts. An instance qualifies State,
//! receipts and coordination; it never bounds or publishes execution.
use super::{authentication::authorize_node, bytes, id, now};
use crate::effect::Effect;
use crate::error::Error;
use crate::execution::NodePrincipal;
use crate::secrets::Sealer;
use meshway_model::state::Capability;
use meshway_model::{InstanceId, NetmapRef, NetworkId, NodeId, Version};
use meshway_protocol::execution::*;
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) const INSTANCE_TTL: i64 = 120_000;
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
) -> Result<(), Error> {
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_execution WHERE network_id=?1 AND node_id=?2 AND instance_id=?3 AND generation=?4 AND boot_id=?5 AND instance_expires_at>?6)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),request.generation.get() as i64,boot.as_uuid().as_bytes().as_slice(),now],|r|r.get(0))?;
    if valid {
        Ok(())
    } else {
        Err(Error::BindingConflict)
    }
}
pub(crate) fn acquire(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: AcquireInstanceRequest,
) -> Result<(InstanceLease, Vec<Effect>), Error> {
    if request.capabilities.len() > 32
        || request.capabilities.iter().any(|(k, v)| {
            k.len() > 64
                || match v {
                    Capability::Transport { roles, streams } => {
                        !k.starts_with("transport:")
                            || roles.is_empty()
                            || roles.len() > 2
                            || streams.is_empty()
                            || streams.len() > 3
                            || roles
                                .iter()
                                .enumerate()
                                .any(|(i, role)| roles[..i].contains(role))
                            || streams
                                .iter()
                                .enumerate()
                                .any(|(i, stream)| streams[..i].contains(stream))
                    }
                    Capability::Feature { version } => !k.starts_with("feature:") || *version == 0,
                }
        })
    {
        return Err(Error::InvalidRequest);
    }
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
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
                return Err(Error::BindingConflict);
            }
            let lease = lease(tx, &p)?;
            return Ok((lease, Vec::new()));
        }
        generation + 1
    } else {
        1
    };
    Version::new(generation.try_into().map_err(|_| Error::BindingConflict)?)
        .map_err(|_| Error::BindingConflict)?;
    let claimed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM node_instance_claims WHERE network_id=?1 AND node_id=?2 AND instance_id=?3)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice()],|r|r.get(0))?;
    if claimed {
        return Err(Error::BindingConflict);
    }
    tx.execute("INSERT INTO node_instance_claims(network_id,node_id,instance_id,generation) VALUES(?1,?2,?3,?4)",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),generation])?;
    // Replacing a process fences its receipt, State and coordination authority.
    tx.execute("INSERT INTO node_execution(network_id,node_id,instance_id,generation,boot_id,instance_expires_at,capabilities) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(network_id,node_id) DO UPDATE SET instance_id=excluded.instance_id,generation=excluded.generation,boot_id=excluded.boot_id,instance_expires_at=excluded.instance_expires_at,capabilities=excluded.capabilities",params![p.network.as_uuid().as_bytes().as_slice(),p.node.as_uuid().as_bytes().as_slice(),request.instance_id.as_uuid().as_bytes().as_slice(),generation,boot.as_uuid().as_bytes().as_slice(),now+INSTANCE_TTL,caps])?;
    let result = lease(tx, &p)?;
    let mut effects = vec![Effect::Progress {
        network_id: p.network,
    }];
    // Authenticated capabilities are trusted inventory: only their change,
    // never the instance itself, reaches the netmap.
    let old_caps = previous.map(|previous| previous.capabilities);
    if old_caps.as_deref() != Some(caps.as_slice())
        && (old_caps.is_some() || !request.capabilities.is_empty())
    {
        effects.extend(
            crate::store::netmap::recompile(
                tx,
                sealer,
                p.network,
                now,
                crate::store::netmap::Trigger::Node,
            )?
            .effects(),
        );
    }
    Ok((result, effects))
}
pub(crate) fn renew(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: InstanceRequest,
) -> Result<(InstanceLease, Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request, now)?;
    tx.execute(
        "UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3",
        params![
            now + INSTANCE_TTL,
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok((lease(tx, &p)?, Vec::new()))
}
pub(crate) fn release(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: InstanceRequest,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request, now)?;
    tx.execute(
        "UPDATE node_execution SET instance_expires_at=?1 WHERE network_id=?2 AND node_id=?3",
        params![
            now,
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice()
        ],
    )?;
    Ok((
        (),
        vec![Effect::Progress {
            network_id: p.network,
        }],
    ))
}
pub(crate) fn lease(tx: &Transaction<'_>, p: &NodePrincipal) -> Result<InstanceLease, Error> {
    lease_for_node(tx, p.network, p.node)
}
pub(crate) fn lease_for_node(
    tx: &Transaction<'_>,
    network: NetworkId,
    node: NodeId,
) -> Result<InstanceLease, Error> {
    let (instance,generation,expires):(Vec<u8>,i64,i64)=tx.query_row("SELECT instance_id,generation,instance_expires_at FROM node_execution WHERE network_id=?1 AND node_id=?2",params![network.as_uuid().as_bytes().as_slice(),node.as_uuid().as_bytes().as_slice()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    Ok(InstanceLease {
        instance_id: id(instance)?,
        generation: Version::new(
            generation
                .try_into()
                .map_err(|_| Error::decode("node_execution.generation"))?,
        )
        .map_err(|_| Error::decode("node_execution.generation"))?,
        expires_at_ms: expires
            .try_into()
            .map_err(|_| Error::decode("node_execution.instance_expires_at"))?,
    })
}
/// The Node held exactly this netmap content at some serial.
fn held(tx: &Transaction<'_>, p: &NodePrincipal, netmap: &NetmapRef) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM node_netmap_history WHERE network_id=?1 AND node_id=?2 AND serial=?3 AND revision=?4 AND digest=?5)",
        params![
            p.network.as_uuid().as_bytes().as_slice(),
            p.node.as_uuid().as_bytes().as_slice(),
            netmap.serial.get() as i64,
            netmap.revision.get() as i64,
            netmap.digest.as_bytes().as_slice()
        ],
        |row| row.get(0),
    )?)
}
pub(crate) fn receipt(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    boot: InstanceId,
    token: [u8; 32],
    request: ReceiptRequest,
) -> Result<((), Vec<Effect>), Error> {
    let now = now()?;
    let p = authorize_node(tx, sealer, boot, token, now)?;
    current_instance(tx, boot, &p, &request.instance, now).map_err(|error| match error {
        Error::BindingConflict => Error::ReceiptInstanceMismatch,
        other => other,
    })?;
    if request.reported_at_ms > now as u64 + 5000
        || request.reported_at_ms.saturating_add(300_000) < now as u64
    {
        return Err(Error::ReceiptPrecondition);
    }
    if (request.phase == ReceiptPhase::Failed) != request.failure.is_some() {
        return Err(Error::ReceiptContentConflict);
    }
    let current = crate::store::netmap::current(tx, p.network, p.node)?.map(|c| c.current);
    if current != Some(request.netmap) {
        // Late evidence for a netmap this Node once held is still history; a
        // netmap it never held is not. Applied names only the current one.
        if !held(tx, &p, &request.netmap)? {
            return Err(Error::ReceiptTargetMismatch);
        }
        if request.phase == ReceiptPhase::Applied {
            return Err(Error::ReceiptNetmapSuperseded);
        }
    }
    let phase = match request.phase {
        ReceiptPhase::Saved => "saved",
        ReceiptPhase::Applied => "applied",
        ReceiptPhase::Failed => "failed",
    };
    if request.phase == ReceiptPhase::Applied {
        let saved: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM netmap_receipts WHERE network_id=?1 AND serial=?2
             AND node_id=?3 AND instance_id=?4 AND attempt=?5 AND sequence<?6 AND phase='saved')",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                request.netmap.serial.get() as i64,
                p.node.as_uuid().as_bytes().as_slice(),
                request.instance.instance_id.as_uuid().as_bytes().as_slice(),
                request.attempt.get() as i64,
                request.sequence.get() as i64
            ],
            |r| r.get(0),
        )?;
        if !saved {
            return Err(Error::ReceiptPrecondition);
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
    let old: Option<(String, i64, Option<String>)> = tx.query_row(
        "SELECT phase,reported_at,failure FROM netmap_receipts
         WHERE network_id=?1 AND serial=?2 AND node_id=?3 AND instance_id=?4 AND attempt=?5 AND sequence=?6",
        params![p.network.as_uuid().as_bytes().as_slice(), request.netmap.serial.get() as i64,
            p.node.as_uuid().as_bytes().as_slice(), request.instance.instance_id.as_uuid().as_bytes().as_slice(),
            request.attempt.get() as i64, request.sequence.get() as i64],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional()?;
    let mut changed = false;
    if let Some(old) = old {
        if old
            != (
                phase.to_owned(),
                request.reported_at_ms as i64,
                failure.map(str::to_owned),
            )
        {
            return Err(Error::ReceiptContentConflict);
        }
    } else {
        // History accepts late evidence; only a new greatest attempt/sequence
        // changes the current progress projection.
        changed = !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM netmap_receipts WHERE network_id=?1 AND serial=?2 AND node_id=?3 AND instance_id=?4 AND (attempt>?5 OR (attempt=?5 AND sequence>?6)))",
            params![p.network.as_uuid().as_bytes().as_slice(), request.netmap.serial.get() as i64, p.node.as_uuid().as_bytes().as_slice(), request.instance.instance_id.as_uuid().as_bytes().as_slice(), request.attempt.get() as i64, request.sequence.get() as i64],
            |row| row.get::<_,bool>(0),
        )?;
        tx.execute(
            "INSERT INTO netmap_receipts(network_id,serial,node_id,instance_id,generation,
             phase,reported_at,received_at,failure,attempt,sequence)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                p.network.as_uuid().as_bytes().as_slice(),
                request.netmap.serial.get() as i64,
                p.node.as_uuid().as_bytes().as_slice(),
                request.instance.instance_id.as_uuid().as_bytes().as_slice(),
                request.instance.generation.get() as i64,
                phase,
                request.reported_at_ms as i64,
                now,
                failure,
                request.attempt.get() as i64,
                request.sequence.get() as i64
            ],
        )?;
    }
    let effects = if changed {
        vec![Effect::Progress {
            network_id: p.network,
        }]
    } else {
        Vec::new()
    };
    Ok(((), effects))
}
