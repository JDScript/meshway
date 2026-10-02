//! Account/ownership installation and strict structural validation.
use crate::Error;
use rusqlite::Transaction;
mod validation;
pub(super) use validation::{present, validate};
const SCHEMA: &str = include_str!("../sql/accounts.sql");

pub(super) fn install(tx: &Transaction<'_>) -> Result<(), Error> {
    tx.execute_batch(SCHEMA)?;
    tx.execute_batch(include_str!("../sql/ownership.sql"))?;
    validate(tx)
}

#[cfg(test)]
mod tests;
