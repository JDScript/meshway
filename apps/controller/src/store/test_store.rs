//! Single-network fixture convenience only. Production Store never has an
//! implicit network: each forwarded call supplies the fixture's validated ID.
use super::{NetworkId, Store};
use crate::{Error, NetworkOptions};
use meshway_protocol::*;
use std::{
    ops::{Deref, DerefMut},
    path::Path,
};

pub(super) const LEGACY_NODE_SELECT: &str = "SELECT id,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,(SELECT revision FROM configuration WHERE id=1),acknowledged_revision FROM nodes";

pub(super) struct TestStore {
    inner: Store,
    pub network_id: NetworkId,
}
impl Deref for TestStore {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.inner
    }
}
impl DerefMut for TestStore {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.inner
    }
}

macro_rules! read {
    ($($name:ident -> $result:ty;)*) => {$(
        pub fn $name(&self)->Result<$result,Error> { self.inner.$name(&self.network_id) }
    )*};
}
macro_rules! write {
    ($($name:ident($arg:ident: $request:ty) -> $result:ty;)*) => {$(
        pub fn $name(&mut self,$arg:$request)->Result<$result,Error> { self.inner.$name(&self.network_id,$arg) }
    )*};
}

impl TestStore {
    pub fn open(path: &Path, options: &NetworkOptions) -> Result<Self, Error> {
        let inner = Store::open(path, options)?;
        let network_id = inner.initial_network_id()?;
        Ok(Self { inner, network_id })
    }
    // Historical tests first populate through the current implementation, then
    // explicitly export the fixed v8 schema before constructing older fixtures.
    pub fn open_v8(path: &Path, options: &NetworkOptions) -> Result<Self, Error> {
        Self::open(path, options)
    }
    read! {
        nodes -> Vec<NodeSummary>;
        count -> u32;
        revision -> u32;
        policy -> PolicySnapshot;
        connections -> ConnectionsSnapshot;
        mesh_access -> MeshAccessSnapshot;
        tags -> TagSnapshot;
        connection_runtimes -> ConnectionRuntimesSnapshot;
        endpoint_discoveries -> EndpointDiscoveriesSnapshot;
    }
    write! {
        revoke(id:&str) -> NodeSummary;
        save_policy(request:&SavePolicyRequest) -> PolicySnapshot;
        set_country(request:&SetNodeCountryRequest) -> NodeSummary;
        save_connections(request:&SaveConnectionsRequest) -> ConnectionsSnapshot;
        save_mesh_access(request:&SaveMeshAccessRequest) -> MeshAccessSnapshot;
        save_tags(request:&SaveTagsRequest) -> TagSnapshot;
        set_node_tags(request:&SetNodeTagsRequest) -> TagSnapshot;
        save_network(request:&SaveNetworkRequest) -> NetworkSnapshot;
    }
    pub fn create_token(&self, ttl: u32) -> Result<CreateEnrollmentTokenResponse, Error> {
        self.inner.create_token(&self.network_id, ttl)
    }
    pub fn authorize_device(
        &self,
        request: &AuthorizeDeviceRequest,
    ) -> Result<AuthorizeDeviceResponse, Error> {
        self.inner.authorize_device(&self.network_id, request)
    }
    pub fn network(&mut self) -> Result<NetworkSnapshot, Error> {
        self.inner.network(&self.network_id)
    }
    pub fn preview(&self, request: &PreviewPolicyRequest) -> Result<PreviewPolicyResponse, Error> {
        self.inner.preview(&self.network_id, request)
    }
    pub fn endpoint_discoveries_at(
        &self,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<EndpointDiscoveriesSnapshot, Error> {
        self.inner.endpoint_discoveries_at(&self.network_id, now)
    }

    pub fn downgrade_v8(&mut self) {
        self.connection.execute_batch("DROP TABLE managed_enrollment_intents; DROP TABLE managed_transport_capabilities; DROP TABLE managed_listeners; DROP INDEX enrollment_tokens_network_id;").unwrap();
        let db = &mut self.inner.connection;
        db.pragma_update(None, "foreign_keys", false).unwrap();
        let tx = db.transaction().unwrap();
        for (table, _) in super::networks::TABLES {
            tx.execute_batch(&format!("ALTER TABLE {table} RENAME TO {table}_v9;"))
                .unwrap();
        }
        tx.execute_batch(include_str!("schema_v8_test.sql"))
            .unwrap();
        for (table, columns) in super::networks::TABLES {
            tx.execute(&format!("INSERT INTO {table} ({columns}) SELECT {columns} FROM {table}_v9 WHERE network_id=?1"),[self.network_id.as_str()]).unwrap();
        }
        for (table, _) in super::networks::TABLES.iter().rev() {
            tx.execute_batch(&format!("DROP TABLE {table}_v9;"))
                .unwrap();
        }
        tx.execute_batch("DROP TABLE networks; PRAGMA user_version=8;")
            .unwrap();
        assert!(
            !tx.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .exists([])
                .unwrap()
        );
        tx.commit().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
    }
    pub fn set_fixture_network_id(&mut self, id: &str) {
        let db = &mut self.inner.connection;
        db.pragma_update(None, "foreign_keys", false).unwrap();
        let tx = db.transaction().unwrap();
        for (table, _) in super::networks::TABLES {
            tx.execute(
                &format!("UPDATE {table} SET network_id=?1 WHERE network_id=?2"),
                rusqlite::params![id, self.network_id.as_str()],
            )
            .unwrap();
        }
        tx.execute(
            "UPDATE networks SET network_id=?1 WHERE network_id=?2",
            rusqlite::params![id, self.network_id.as_str()],
        )
        .unwrap();
        tx.commit().unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        self.network_id = self.inner.network_id(id).unwrap();
    }
}

pub(super) fn original_columns(db: &rusqlite::Connection, table: &str) -> String {
    db.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|name| name != "network_id" || table == "network_identity")
        .collect::<Vec<_>>()
        .join(",")
}
