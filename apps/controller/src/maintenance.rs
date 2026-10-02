//! Explicit offline administration. No HTTP handlers or automatic recovery.
use crate::{Error, NetworkOptions, store::Store};
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

/// Takes the controller's exclusive directory lock; private input and errors
/// never expose secret values or file contents.
pub fn import_managed_vless(
    data_dir: &Path,
    input: &Path,
) -> Result<meshway_protocol::ManagedListenersSnapshot, Error> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Import {
        network_id: String,
        legacy_profiles: std::path::PathBuf,
        request: meshway_protocol::SaveManagedListenerRequest,
        material: meshway_protocol::ManagedVlessMaterial,
    }
    let bytes = zeroize::Zeroizing::new(local::read_private_bounded(input, 256 * 1024)?);
    let import: Import = serde_json::from_str(&bytes)
        .map_err(|_| Error::Invalid("invalid private managed VLESS import"))?;
    let mut store = Store::open(data_dir, &NetworkOptions::default())?;
    let network = store.network_id(&import.network_id)?;
    store.verify_legacy_material(
        &network,
        &import.request.settings,
        &import.material,
        &import.legacy_profiles,
    )?;
    store.import_managed_listener(&network, &import.request, import.material)
}
