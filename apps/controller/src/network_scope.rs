//! Validated Controller network scope, shared by storage and accounts.
//! Existence/identity is not membership or management authorization.
use crate::Error;
use meshway_protocol::local;
use rusqlite::{Connection, OptionalExtension, Row};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NetworkId(String);

impl NetworkId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    fn stored(value: String) -> Result<Self, Error> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Configuration("invalid persisted network identity"));
        }
        Ok(Self(value))
    }

    pub(crate) fn lookup(connection: &Connection, id: &str) -> Result<Self, Error> {
        Self::stored(
            connection
                .query_row(
                    "SELECT network_id FROM networks WHERE network_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(Error::NotFound)?,
        )
    }

    pub(crate) fn initial(connection: &Connection) -> Result<Self, Error> {
        Self::stored(connection.query_row(
            "SELECT network_id FROM networks WHERE initial=1",
            [],
            |row| row.get(0),
        )?)
    }

    pub(crate) fn all(connection: &Connection) -> Result<Vec<Self>, Error> {
        let ids = connection
            .prepare("SELECT network_id FROM networks ORDER BY initial DESC,created_at,network_id")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter().map(Self::stored).collect()
    }

    // Reads persisted identity evidence; no arbitrary string constructor.
    pub(crate) fn read(row: &Row<'_>, column: usize) -> Result<Self, Error> {
        Self::stored(row.get(column)?)
    }

    // A fresh ID is generated only for insertion as a new network.
    pub(crate) fn generate() -> Result<Self, Error> {
        Ok(Self(local::credential()?[..32].to_owned()))
    }

    pub(crate) fn node_credential(connection: &Connection, token: &str) -> Result<Self, Error> {
        Self::credential(
            connection,
            token,
            "SELECT network_id FROM nodes WHERE credential_hash=?1",
        )
    }
    pub(crate) fn enrollment_credential(
        connection: &Connection,
        token: &str,
    ) -> Result<Self, Error> {
        Self::credential(
            connection,
            token,
            "SELECT network_id FROM enrollment_tokens WHERE token_hash=?1",
        )
    }
    fn credential(connection: &Connection, token: &str, sql: &str) -> Result<Self, Error> {
        Self::stored(
            connection
                .query_row(sql, [local::digest(token)], |row| row.get(0))
                .optional()?
                .ok_or(Error::Unauthorized)?,
        )
    }
}
