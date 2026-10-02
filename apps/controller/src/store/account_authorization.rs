//! Root transaction authorization for existing network APIs. No ambient network.
use super::{NetworkId, Store};
use crate::{
    Error,
    accounts::{Accounts, NetworkPermission, Principal},
};
use rusqlite::{Connection, Savepoint, Transaction, TransactionBehavior};
use std::ops::Deref;

/// Business operations nested in a checked root use a savepoint; direct calls
/// retain their original IMMEDIATE/DEFERRED transaction semantics.
pub(super) enum BusinessTransaction<'a> {
    Root(Transaction<'a>),
    Nested(Savepoint<'a>),
}
impl Deref for BusinessTransaction<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        match self {
            Self::Root(tx) => tx,
            Self::Nested(tx) => tx,
        }
    }
}
impl BusinessTransaction<'_> {
    pub(super) fn commit(self) -> Result<(), rusqlite::Error> {
        match self {
            Self::Root(tx) => tx.commit(),
            Self::Nested(tx) => tx.commit(),
        }
    }
}
pub(super) fn transaction(
    db: &mut Connection,
    behavior: TransactionBehavior,
) -> rusqlite::Result<BusinessTransaction<'_>> {
    if db.is_autocommit() {
        Ok(BusinessTransaction::Root(
            db.transaction_with_behavior(behavior)?,
        ))
    } else {
        Ok(BusinessTransaction::Nested(db.savepoint()?))
    }
}

impl Store {
    pub(super) fn principal(&self, token: &str, now: i64) -> Result<Principal, Error> {
        Ok(Principal::authenticate_source(
            &self.connection,
            token,
            now,
        )?)
    }

    /// BEGIN IMMEDIATE precedes role/session lookup, and is held through the
    /// actual mutation, including its nested savepoint and publication checks.
    pub(crate) fn network_operation<T>(
        &mut self,
        token: &str,
        id: &str,
        permission: NetworkPermission,
        action: impl FnOnce(&mut Self, &NetworkId) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.principal_operation(token, |store, principal, now| {
            let network = store.network_id(id)?;
            Accounts::new(&store.connection)
                .authorize_network(principal, &network, permission, now)?;
            action(store, &network)
        })
    }

    fn principal_operation<T>(
        &mut self,
        token: &str,
        action: impl FnOnce(&mut Self, &Principal, i64) -> Result<T, Error>,
    ) -> Result<T, Error> {
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        let outcome = (|| {
            let now = chrono::Utc::now().timestamp();
            let principal = self.principal(token, now)?;
            action(self, &principal, now)
        })();
        match outcome {
            Ok(value) => {
                if let Err(error) = self.connection.execute_batch("COMMIT") {
                    let _ = self.connection.execute_batch("ROLLBACK");
                    return Err(error.into());
                }
                Ok(value)
            }
            Err(error) => {
                self.connection.execute_batch("ROLLBACK")?;
                Err(error)
            }
        }
    }

    pub(crate) fn authorized_networks(
        &mut self,
        token: &str,
    ) -> Result<meshway_protocol::NetworkList, Error> {
        self.principal_operation(token, |store, principal, now| {
            if !token.starts_with("mws_") {
                return store.networks();
            }
            let memberships = Accounts::new(&store.connection).memberships(principal, now)?;
            Ok(meshway_protocol::NetworkList {
                networks: memberships
                    .into_iter()
                    .map(|m| store.network(&m.network))
                    .collect::<Result<_, _>>()?,
            })
        })
    }
}
