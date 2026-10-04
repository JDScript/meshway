use crate::error::Error;
use crate::secrets::{Plaintext, ProtectedBytes, Purpose, Sealer};
use meshway_model::{NetworkId, RequestId, Revision, UserId};
use meshway_protocol::configuration::ConfigurationAppliedResponse;
use rusqlite::{OptionalExtension, Transaction, params};

pub(crate) struct StoredCommit {
    pub(crate) original: Plaintext,
    result: String,
}

impl StoredCommit {
    pub(crate) fn result(self) -> Result<ConfigurationAppliedResponse, Error> {
        serde_json::from_str(&self.result)
            .map_err(|_| Error::decode("configuration_commits.result"))
    }
}

pub(crate) fn find(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    user: UserId,
    request: RequestId,
) -> Result<Option<StoredCommit>, Error> {
    let prior = tx.query_row(
        "SELECT base_revision,input,result FROM configuration_commits WHERE network_id=?1 AND user_id=?2 AND request_id=?3",
        params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(), request.as_uuid().as_bytes().as_slice()],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, String>(2)?)),
    ).optional()?;
    prior
        .map(|(base, input, result)| {
            let base = Revision::new(
                u64::try_from(base)
                    .map_err(|_| Error::decode("configuration_commits.base_revision"))?,
            )
            .map_err(|_| Error::decode("configuration_commits.base_revision"))?;
            let original = sealer.open(network, base, Purpose::Request, &input)?;
            Ok(StoredCommit { original, result })
        })
        .transpose()
}

pub(crate) fn insert(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    request: RequestId,
    base: Revision,
    input: &ProtectedBytes,
    result: &ConfigurationAppliedResponse,
) -> Result<(), Error> {
    tx.execute(
        "INSERT INTO configuration_commits(network_id,user_id,request_id,base_revision,input,result) VALUES(?1,?2,?3,?4,?5,?6)",
        params![network.as_uuid().as_bytes().as_slice(), user.as_uuid().as_bytes().as_slice(),
            request.as_uuid().as_bytes().as_slice(), base.get() as i64,
            input.as_bytes(), serde_json::to_string(result).map_err(|_|Error::encode("configuration commit result"))?],
    )?;
    Ok(())
}

pub(crate) fn exists(
    tx: &Transaction<'_>,
    network: NetworkId,
    user: UserId,
    request: RequestId,
) -> Result<bool, Error> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM configuration_commits WHERE network_id=?1 AND user_id=?2 AND request_id=?3)",
        params![network.as_uuid().as_bytes().as_slice(),user.as_uuid().as_bytes().as_slice(),request.as_uuid().as_bytes().as_slice()],
        |row| row.get::<_,bool>(0),
    )?)
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::error::Error;
    use rusqlite::Transaction;

    pub(crate) fn install_failure(tx: &Transaction<'_>) -> Result<(), Error> {
        tx.execute_batch("CREATE TRIGGER fail_commit BEFORE INSERT ON configuration_commits BEGIN SELECT RAISE(ABORT, 'injected failure'); END;")?;
        Ok(())
    }

    pub(crate) fn counts(tx: &Transaction<'_>) -> Result<(i64, i64), Error> {
        let versions = tx.query_row("SELECT count(*) FROM configuration_versions", [], |r| {
            r.get(0)
        })?;
        let activities = tx.query_row(
            "SELECT count(*) FROM activity_events WHERE action='configuration.apply'",
            [],
            |r| r.get(0),
        )?;
        Ok((versions, activities))
    }

    pub(crate) fn remove_failure(tx: &Transaction<'_>) -> Result<(), Error> {
        tx.execute_batch("DROP TRIGGER fail_commit")?;
        Ok(())
    }
}
