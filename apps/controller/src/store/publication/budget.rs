//! Serialized inventory and adjacency publication budgets.
use super::super::NetworkId;
use crate::Error;
use meshway_protocol::*;
use rusqlite::Connection;

pub(in crate::store) fn configuration(snapshot: ConfigurationSnapshot) -> Result<(), Error> {
    let bytes = serde_json::to_vec(&snapshot)
        .map_err(|_| Error::Configuration("cannot encode configuration"))?;
    if bytes.len() > MAX_CONFIGURATION_BYTES {
        return Err(Error::Capacity(
            "configuration exceeds 1 MiB publication budget",
        ));
    }
    Ok(())
}

pub(in crate::store) fn connections(
    db: &Connection,
    scope: &NetworkId,
    projected: &AgentConnectionsSnapshot,
) -> Result<(), Error> {
    let encoded = serde_json::to_vec(projected)
        .map_err(|_| Error::Configuration("cannot encode connections"))?;
    let private_budget =
        super::super::managed_transports::material_budget(db, scope, &projected.payload)?;
    if encoded.len().saturating_add(private_budget) > MAX_CONNECTIONS_BYTES - 4096 {
        return Err(Error::Capacity(
            "connections exceed 1 MiB publication budget",
        ));
    }
    Ok(())
}
