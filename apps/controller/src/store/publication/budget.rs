//! Publication limits reserve the same headroom as the legacy write gates.
use super::*;

pub(in crate::store) fn configuration(mut snapshot: ConfigurationSnapshot) -> Result<(), Error> {
    // Revocation can add diagnostics; reserve their bounded report in advance.
    snapshot.payload.diagnostics.clear();
    let bytes = serde_json::to_vec(&snapshot)
        .map_err(|_| Error::Configuration("cannot encode configuration"))?;
    if bytes.len() > MAX_CONFIGURATION_BYTES - configuration::DIAGNOSTIC_BUDGET {
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
