//! SQL behind the master key binding.
use crate::{error::Error, secrets::ProtectedBytes};
use meshway_model::KeyId;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) struct Binding {
    pub(crate) key_matches: bool,
    pub(crate) proof: Vec<u8>,
}

pub(crate) fn binding(connection: &Connection, key: KeyId) -> Result<Option<Binding>, Error> {
    let binding: Option<(Vec<u8>, Vec<u8>)> = connection
        .query_row(
            "SELECT key_id, key_check FROM master_key_binding WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(binding.map(|(id, proof)| Binding {
        key_matches: id == key.as_uuid().as_bytes(),
        proof,
    }))
}

pub(crate) fn insert_binding(
    connection: &Connection,
    key: KeyId,
    proof: &ProtectedBytes,
) -> Result<(), Error> {
    connection.execute(
        "INSERT INTO master_key_binding (singleton, key_id, key_check) VALUES (1, ?1, ?2)",
        params![key.as_uuid().as_bytes().as_slice(), proof.as_bytes()],
    )?;
    Ok(())
}
