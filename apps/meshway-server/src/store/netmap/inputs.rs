//! Incremental compilation hints, atomically replaced with their outputs.
//! These hashes cannot supply a netmap or authorize any request.
use super::*;

pub(super) fn load(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<BTreeMap<NodeId, Digest>, Error> {
    let mut statement =
        tx.prepare("SELECT node_id,digest FROM node_netmap_inputs WHERE network_id=?1")?;
    let mut rows = statement.query([bytes(network.as_uuid())])?;
    let mut inputs = BTreeMap::new();
    while let Some(row) = rows.next()? {
        inputs.insert(id(row.get(0)?)?, digest(row.get(1)?)?);
    }
    Ok(inputs)
}

pub(super) fn save(
    tx: &Transaction<'_>,
    network: NetworkId,
    before: &BTreeMap<NodeId, Digest>,
    after: &BTreeMap<NodeId, Digest>,
) -> Result<(), Error> {
    for (node, digest) in after {
        if before.get(node) != Some(digest) {
            tx.execute("INSERT INTO node_netmap_inputs(network_id,node_id,digest) VALUES(?1,?2,?3) ON CONFLICT(network_id,node_id) DO UPDATE SET digest=excluded.digest",
                params![bytes(network.as_uuid()), bytes(node.as_uuid()), digest.as_bytes().as_slice()])?;
        }
    }
    for node in before.keys().filter(|node| !after.contains_key(node)) {
        tx.execute(
            "DELETE FROM node_netmap_inputs WHERE network_id=?1 AND node_id=?2",
            params![bytes(network.as_uuid()), bytes(node.as_uuid())],
        )?;
    }
    Ok(())
}
