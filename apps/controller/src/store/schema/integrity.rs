use crate::{Error, store::NetworkId};
use rusqlite::Connection;
/// Includes revoked identities, semantic address duplicates and FK integrity.
pub(super) fn validate_integrity(connection: &Connection) -> Result<(), Error> {
    use std::{
        collections::BTreeSet,
        net::{Ipv4Addr, Ipv6Addr},
    };
    let mut addresses4 = BTreeSet::new();
    let mut addresses6 = BTreeSet::new();
    let mut query = connection.prepare("SELECT n.network_id,n.mesh_ipv4,n.mesh_ipv6,c.ipv4_prefix,c.ipv6_prefix FROM nodes n LEFT JOIN configuration c ON c.network_id=n.network_id AND c.id=1 ORDER BY n.id")?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let network = NetworkId::read(row, 0)?;
        let invalid =
            || Error::Configuration("node has invalid or missing persistent mesh addresses");
        let ipv4: Ipv4Addr = row
            .get::<_, Option<String>>(1)?
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let ipv6: Ipv6Addr = row
            .get::<_, Option<String>>(2)?
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let pool = meshway_protocol::MeshNetwork {
            ipv4_prefix: row.get::<_, Option<String>>(3)?.ok_or_else(invalid)?,
            ipv6_prefix: row.get::<_, Option<String>>(4)?.ok_or_else(invalid)?,
        };
        let (v4, v6) = crate::store::ipam::parse(&pool)?;
        if !v4.contains(&ipv4)
            || ipv4 == v4.network()
            || ipv4 == v4.broadcast()
            || ipv4.is_unspecified()
            || ipv4.is_loopback()
            || ipv4.is_multicast()
            || ipv4.is_link_local()
            || !v6.contains(&ipv6)
            || ipv6 == v6.network()
            || !addresses4.insert((network.as_str().to_owned(), ipv4))
            || !addresses6.insert((network.as_str().to_owned(), ipv6))
        {
            return Err(invalid());
        }
    }
    if connection.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "persisted network relationships fail foreign key validation",
        ));
    }
    Ok(())
}
