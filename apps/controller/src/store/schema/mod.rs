//! One current schema. Startup validates existing state and never repairs or
//! clears an unsupported database; development resets are deployment operations.
mod integrity;
use crate::{Error, NetworkOptions};
use meshway_protocol::{MeshDnsConfiguration, MeshNetwork, local};
use rusqlite::{Connection, TransactionBehavior, params};
use std::{collections::BTreeMap, path::Path};
use zeroize::Zeroizing;

pub(super) const VERSION: i64 = 14;
const SQL: &str = include_str!("schema.sql");

fn objects(db: &Connection) -> Result<BTreeMap<(String, String, String), String>, Error> {
    Ok(db.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name")?
        .query_map([], |r| Ok(((r.get(0)?,r.get(1)?,r.get(2)?),r.get(3)?)))?.collect::<Result<_, _>>()?)
}

pub(in crate::store) fn validate(db: &Connection) -> Result<(), Error> {
    let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != VERSION {
        return Err(Error::Configuration(
            "unsupported or incomplete controller database schema; explicit development reset required",
        ));
    }
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(SQL)?;
    if objects(db)? != objects(&reference)?
        || db
            .prepare("SELECT 1 FROM sqlite_temp_schema WHERE sql IS NOT NULL")?
            .exists([])?
    {
        return Err(Error::Configuration(
            "invalid current controller schema; refusing repair",
        ));
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(Error::Configuration("corrupt controller database"));
    }
    integrity::validate_integrity(db)?;
    if db.prepare("SELECT n.id FROM nodes n LEFT JOIN node_ownership o ON o.node_id=n.id AND o.network_id=n.network_id GROUP BY n.network_id,n.id HAVING COUNT(o.node_id)<>1")?.exists([])? {
        return Err(Error::Configuration("incomplete node ownership binding"));
    }
    // Every network owns its current policy and singleton configuration rows.
    if db.query_row("SELECT EXISTS(SELECT 1 FROM networks n LEFT JOIN network_policy p ON p.network_id=n.network_id LEFT JOIN configuration c ON c.network_id=n.network_id LEFT JOIN network_identity i ON i.network_id=n.network_id LEFT JOIN connection_source s ON s.network_id=n.network_id WHERE p.network_id IS NULL OR c.network_id IS NULL OR i.network_id IS NULL OR s.network_id IS NULL)",[],|r|r.get::<_,bool>(0))? {
        return Err(Error::Configuration("incomplete network configuration"));
    }
    for network in super::NetworkId::all(db)? {
        super::network_policy::source(db, &network)?;
    }
    Ok(())
}

pub(super) fn initialize(
    db: &mut Connection,
    path: &Path,
    new: bool,
    admin: &str,
    options: &NetworkOptions,
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if new {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(SQL)?;
        tx.execute("INSERT INTO settings VALUES(1,?1)", [local::digest(admin)])?;
        let id = super::NetworkId::generate()?;
        let random = local::credential()?;
        let pool = MeshNetwork {
            ipv4_prefix: options
                .ipv4_prefix
                .clone()
                .unwrap_or_else(|| "100.96.0.0/16".into()),
            ipv6_prefix: options.ipv6_prefix.clone().unwrap_or_else(|| {
                format!(
                    "fd{}:{}:{}::/64",
                    &random[..2],
                    &random[2..6],
                    &random[6..10]
                )
            }),
        };
        let (v4, v6) = super::ipam::parse(&pool)?;
        let dns = serde_json::to_string(&MeshDnsConfiguration {
            enabled: false,
            zone: format!("net-{}.meshway.test", id.as_str()),
            ttl_seconds: 30,
        })
        .map_err(|_| Error::Configuration("cannot encode DNS"))?;
        tx.execute(
            "INSERT INTO networks VALUES(?1,?2,1)",
            params![id.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        tx.execute("INSERT INTO configuration(network_id,id,revision,ipv4_prefix,ipv6_prefix) VALUES(?1,1,1,?2,?3)",params![id.as_str(),v4.to_string(),v6.to_string()])?;
        tx.execute(
            "INSERT INTO network_identity VALUES(1,?1,'Meshway',?2)",
            params![id.as_str(), dns],
        )?;
        tx.execute(
            "INSERT INTO connection_source VALUES(?1,1,'{\"listeners\":[],\"candidates\":[]}')",
            [id.as_str()],
        )?;
        super::network_policy::initialize(&tx, &id)?;
        validate(&tx)?;
        // A key file is private and durable before publishing the database.
        let key = super::managed_transports::key(path, &tx)?;
        tx.commit()?;
        local::ensure_durable(&path.join("controller.sqlite3"))?;
        Ok(key)
    } else {
        validate(db)?;
        let stored: Vec<u8> =
            db.query_row("SELECT admin_hash FROM settings WHERE id=1", [], |r| {
                r.get(0)
            })?;
        if stored != local::digest(admin) {
            return Err(Error::Configuration(
                "admin.token does not match the database; refusing to reinitialize",
            ));
        }
        super::managed_transports::key(path, db)
    }
}

#[cfg(test)]
mod tests;
