//! Unversioned P2b migration hook. The integrating release owns user_version.
use crate::{Error, accounts};
use rusqlite::Transaction;
mod validation;
pub(super) use validation::{present, validate};

pub(crate) fn install(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch(accounts::SCHEMA_FRAGMENT)?;
    tx.execute_batch(include_str!("ownership.sql"))?;
    validate(tx)
}

#[cfg(test)]
mod tests;
