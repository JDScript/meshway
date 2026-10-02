//! Explicit offline administration. No HTTP handlers or automatic recovery.
use crate::Error;
use meshway_protocol::local;
use std::path::Path;

/// Reads the existing private bootstrap file, including while the server runs.
/// Does not initialize storage or replace an identity.
pub fn admin_token(data_dir: impl AsRef<Path>) -> Result<String, Error> {
    let data_dir = data_dir.as_ref();
    std::fs::symlink_metadata(data_dir)?;
    local::private_dir(data_dir)?;
    local::open_private(&data_dir.join("controller.sqlite3"), false)?;
    let token = local::read_private(&data_dir.join("admin.token"))?;
    let token = token.trim_end_matches('\n');
    if !local::valid_credential(token) {
        return Err(Error::Configuration("invalid admin.token"));
    }
    let connection = rusqlite::Connection::open_with_flags(
        data_dir.join("controller.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let stored: Vec<u8> =
        connection.query_row("SELECT admin_hash FROM settings WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    if stored != local::digest(token) {
        return Err(Error::Configuration(
            "admin.token does not match the database",
        ));
    }
    Ok(token.to_owned())
}
