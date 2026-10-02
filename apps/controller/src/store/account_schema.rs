//! Unversioned P2b migration hook. The integrating release owns user_version.
use crate::{Error, accounts};
use rusqlite::Transaction;

pub(crate) fn install(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch(accounts::SCHEMA_FRAGMENT)?;
    tx.execute_batch(include_str!("ownership.sql"))?;
    if tx.prepare("PRAGMA foreign_key_check")?.exists([])? {
        return Err(Error::Configuration(
            "account migration foreign key failure",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
