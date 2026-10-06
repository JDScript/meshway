use super::{CircuitAuthorization, Error, crypto};
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::NodeConfiguration;
use meshway_model::{Digest, NodeId};
use meshway_policy::admission::{admit_circuit_handshake, circuit_permit};

pub(super) fn digest(
    cfg: &NodeConfiguration,
    authorization: CircuitAuthorization,
) -> Result<Digest, Error> {
    match authorization {
        CircuitAuthorization::Business(id) => {
            meshway_policy::permit_authorization_digest(cfg.permits.get(&id).ok_or(Error::Context)?)
        }
        CircuitAuthorization::Diagnostic(id) => meshway_policy::diagnostic_authorization_digest(
            cfg.diagnostics.get(&id).ok_or(Error::Context)?,
        ),
    }
    .map_err(|_| Error::Context)
}

pub(super) fn source_instance(
    snapshot: &ExecutionSnapshot,
    authorization: CircuitAuthorization,
) -> Result<(), Error> {
    if let CircuitAuthorization::Diagnostic(id) = authorization {
        let cfg = snapshot.configuration().map_err(|_| Error::Authority)?;
        let permit = cfg.diagnostics.get(&id).ok_or(Error::Context)?;
        if permit.source == cfg.node_id && permit.instance_id != snapshot.instance().instance_id {
            return Err(Error::Context);
        }
    }
    Ok(())
}

pub(super) fn unchanged(
    old: &NodeConfiguration,
    new: &NodeConfiguration,
    authorization: CircuitAuthorization,
    path: &[NodeId],
    now_ms: u64,
) -> Result<bool, Error> {
    if let CircuitAuthorization::Business(id) = authorization {
        return crate::flow::authorization_unchanged(old, new, id, path, now_ms)
            .map_err(|_| Error::Context);
    }
    let (Ok(before), Ok(after)) = (
        circuit_permit(old, authorization),
        circuit_permit(new, authorization),
    ) else {
        return Ok(false);
    };
    if now_ms >= before.expires_at_ms
        || now_ms >= after.expires_at_ms
        || digest(old, authorization)? != digest(new, authorization)?
        || crypto::key_digest(old, path)? != crypto::key_digest(new, path)?
        || path
            .iter()
            .any(|node| *node != old.node_id && old.peers.get(node) != new.peers.get(node))
    {
        return Ok(false);
    }
    let Some(selected) = before
        .paths
        .iter()
        .find(|candidate| candidate.nodes == path)
    else {
        return Ok(false);
    };
    let position = path
        .iter()
        .position(|node| *node == new.node_id)
        .ok_or(Error::Context)?;
    if admit_circuit_handshake(
        new,
        authorization,
        path,
        before.source,
        position.checked_sub(1).map(|i| path[i]),
        false,
        now_ms,
    )
    .is_err()
    {
        return Ok(false);
    }
    let prefix = format!("{}/", old.node_id);
    for (dialer, keys) in &selected.dial_candidates {
        for key in keys {
            if *dialer == old.node_id && old.dialers.get(key) != new.dialers.get(key) {
                return Ok(false);
            }
            if let Some(listener) = key.strip_prefix(&prefix)
                && old.listeners.get(listener) != new.listeners.get(listener)
            {
                return Ok(false);
            }
        }
    }
    Ok(true)
}
