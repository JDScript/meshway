use crate::accounts::random_uuid;
use crate::configuration::{ConfigurationBindings, PreparedConfiguration, Snapshot};
use crate::error::Error;
use crate::secrets::{ProtectedBytes, Purpose, Sealer};
use meshway_configuration::ConfigurationDocument;
use meshway_model::{NetmapRef, NetworkId, NodeId, RequestId, Revision, UserId};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;

fn current_revision_value(tx: &Transaction<'_>, network: NetworkId) -> Result<i64, Error> {
    tx.query_row(
        "SELECT current_revision FROM networks WHERE id=?1 AND deleted_at IS NULL",
        [network.as_uuid().as_bytes().as_slice()],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .ok_or(Error::NotFound)
}

pub(crate) fn current_revision(
    tx: &Transaction<'_>,
    network: NetworkId,
) -> Result<Revision, Error> {
    let r = current_revision_value(tx, network)?;
    Revision::new(u64::try_from(r).map_err(|_| Error::decode("networks.current_revision"))?)
        .map_err(|_| Error::decode("networks.current_revision"))
}

pub(crate) fn current_revision_matches(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<bool, Error> {
    Ok(current_revision_value(tx, network)? == revision.get() as i64)
}

pub(crate) struct VersionCommit {
    pub(crate) network: NetworkId,
    pub(crate) previous_revision: Revision,
    pub(crate) revision: Revision,
    pub(crate) actor: UserId,
    pub(crate) request: RequestId,
    pub(crate) now: i64,
}

pub(crate) fn insert_version(
    tx: &Transaction<'_>,
    commit: &VersionCommit,
    prepared: &PreparedConfiguration,
    sealed: [&ProtectedBytes; 3],
) -> Result<(), Error> {
    let VersionCommit {
        network,
        previous_revision,
        revision,
        actor,
        request,
        now,
    } = commit;
    let [source, configuration, bindings] = sealed;
    tx.execute(
        "INSERT INTO configuration_versions(network_id,revision,source_jsonc,configuration,bindings,created_by,created_at,cause,request_id,rollback_from,created_by_email) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,(SELECT email FROM users WHERE id=?6))",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64,
            source.as_bytes(), configuration.as_bytes(), bindings.as_bytes(),
            actor.as_uuid().as_bytes().as_slice(), now, prepared.cause.as_str(),
            request.as_uuid().as_bytes().as_slice(), prepared.rollback_from.map(|r|r.get() as i64)],
    )?;
    tx.execute(
        "UPDATE networks SET current_revision=?1 WHERE id=?2 AND current_revision=?3",
        params![
            revision.get() as i64,
            network.as_uuid().as_bytes().as_slice(),
            previous_revision.get() as i64
        ],
    )?;
    Ok(())
}

pub(crate) fn record_execution_targets(
    tx: &Transaction<'_>,
    commit: &VersionCommit,
    execution_targets: &BTreeMap<NodeId, NetmapRef>,
) -> Result<(), Error> {
    let VersionCommit {
        network,
        previous_revision,
        revision,
        actor,
        request,
        now,
    } = commit;
    tx.execute(
        "UPDATE configuration_versions SET execution_targets=?3 WHERE network_id=?1 AND revision=?2",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64,
            serde_json::to_string(execution_targets).map_err(|_| Error::encode("configuration execution targets"))?],
    )?;
    tx.execute(
        "INSERT INTO activity_events(id,scope,network_id,at,actor_user_id,action,request_id,before_revision,after_revision,actor_email) VALUES(?1,'network',?2,?3,?4,'configuration.apply',?5,?6,?7,(SELECT email FROM users WHERE id=?4))",
        params![random_uuid()?.as_bytes().as_slice(),
            network.as_uuid().as_bytes().as_slice(), now,
            actor.as_uuid().as_bytes().as_slice(), request.as_uuid().as_bytes().as_slice(),
            previous_revision.get() as i64, revision.get() as i64],
    )?;
    Ok(())
}

/// The sealed bindings of every version a network that is not deleted can
/// restore, opened by `open`.
pub(crate) fn each_restorable_bindings(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    mut each: impl FnMut(ConfigurationBindings) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut statement = tx.prepare(
        "SELECT v.network_id,v.revision,v.bindings FROM configuration_versions v
         JOIN networks n ON n.id=v.network_id WHERE n.deleted_at IS NULL",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let network = crate::store::execution::id(row.get(0)?)?;
        let revision = Revision::new(
            u64::try_from(row.get::<_, i64>(1)?)
                .map_err(|_| Error::decode("configuration_versions.revision"))?,
        )
        .map_err(|_| Error::decode("configuration_versions.revision"))?;
        let bindings = sealer.open(
            network,
            revision,
            Purpose::Bindings,
            row.get_ref(2)?
                .as_blob()
                .map_err(|_| Error::decode("configuration_versions.bindings"))?,
        )?;
        each(
            serde_json::from_slice(bindings.as_bytes())
                .map_err(|_| Error::decode("configuration_versions.bindings"))?,
        )?;
    }
    Ok(())
}

pub(crate) struct SealedVersion {
    network: NetworkId,
    revision: Revision,
    source: Vec<u8>,
    configuration: Vec<u8>,
    bindings: Vec<u8>,
}

pub(crate) fn load(
    tx: &Transaction<'_>,
    network: NetworkId,
    revision: Revision,
) -> Result<SealedVersion, Error> {
    let (source, configuration, bindings) = tx.query_row(
        "SELECT source_jsonc,configuration,bindings FROM configuration_versions WHERE network_id=?1 AND revision=?2",
        params![network.as_uuid().as_bytes().as_slice(), revision.get() as i64],
        |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)),
    ).optional()?.ok_or(Error::NotFound)?;
    Ok(SealedVersion {
        network,
        revision,
        source,
        configuration,
        bindings,
    })
}

impl SealedVersion {
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        for bytes in [&self.source, &self.configuration, &self.bindings] {
            hash.update((bytes.len() as u64).to_be_bytes());
            hash.update(bytes);
        }
        hash.finalize().into()
    }

    pub(crate) fn encoded_size(&self) -> usize {
        self.source.len() + self.configuration.len() + self.bindings.len()
    }

    pub(crate) fn open(self, sealer: &Sealer) -> Result<Snapshot, Error> {
        let Self {
            network,
            revision,
            source,
            configuration,
            bindings,
        } = self;
        let source = sealer.open(network, revision, Purpose::Source, &source)?;
        let configuration =
            sealer.open(network, revision, Purpose::Configuration, &configuration)?;
        let bindings = sealer.open(network, revision, Purpose::Bindings, &bindings)?;
        let source = std::str::from_utf8(source.as_bytes())
            .map_err(|_| Error::decode("configuration_versions.source_jsonc"))?;
        meshway_configuration::parse_jsonc(source)
            .map_err(|_| Error::decode("configuration_versions.source_jsonc"))?;
        let configuration = meshway_configuration::restore_canonical_json(
            std::str::from_utf8(configuration.as_bytes())
                .map_err(|_| Error::decode("configuration_versions.configuration"))?,
        )
        .map_err(|_| Error::decode("configuration_versions.configuration"))?;
        let bindings: ConfigurationBindings = serde_json::from_slice(bindings.as_bytes())
            .map_err(|_| Error::decode("configuration_versions.bindings"))?;
        Ok(Snapshot {
            network_id: network,
            revision,
            document: ConfigurationDocument::new(source.to_owned(), configuration),
            bindings,
        })
    }
}
