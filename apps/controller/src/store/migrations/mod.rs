//! Sole owner of schema installation, the complete 1 -> 12 path and startup
//! structural validation. Identity files are established by Store before entry.
mod accounts;
pub(in crate::store) mod execution_and_accounts;
pub(in crate::store) mod latency;
pub(in crate::store) mod legacy;
mod mesh_policy;
pub(in crate::store) mod networks;
pub(in crate::store) mod transport_materials;
pub(super) use execution_and_accounts::VERSION;

use crate::{Error, NetworkOptions};
use meshway_protocol::local;
use rusqlite::{Connection, TransactionBehavior};
use std::{fs::File, path::Path};
use zeroize::Zeroizing;

pub(super) fn initialize(
    connection: &mut Connection,
    path: &Path,
    new: bool,
    admin: &str,
    options: &NetworkOptions,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if new {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(include_str!("sql/schema_v1.sql"))?;
        transaction.execute(
            "INSERT INTO settings VALUES (1, ?1)",
            [local::digest(admin)],
        )?;
        transaction.commit()?;
        File::open(path)?.sync_all()?;
    }
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(1..=VERSION).contains(&version) {
        return Err(Error::Configuration(
            "unsupported or incomplete controller database schema",
        ));
    }
    let stored: Vec<u8> =
        connection.query_row("SELECT admin_hash FROM settings WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    if stored != local::digest(admin) {
        return Err(Error::Configuration(
            "admin.token does not match the database; refusing to reinitialize",
        ));
    }
    // The original version controls dispatch; individual steps retain their
    // version gates and original transaction boundaries.
    if version < 9 {
        legacy::upgrade(connection, options, version)?;
        networks::upgrade(connection)?;
    }
    if version < 10 {
        transport_materials::upgrade(connection)?;
    }
    // Existing material must not be rebound to a replacement local key.
    // Keep key recovery before global integrity checks and release installation.
    let material_key = super::managed_transports::key(path, connection)?;
    networks::validate_integrity(connection)?;
    networks::install_identity_guards(connection)?;
    execution_and_accounts::upgrade(connection)?;
    Ok(material_key)
}
