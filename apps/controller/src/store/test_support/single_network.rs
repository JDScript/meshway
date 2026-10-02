//! Single-network fixture convenience only. Production Store never has an
//! implicit network: each forwarded call supplies the fixture's validated ID.
use crate::store::{NetworkId, Store};
use crate::{Error, NetworkOptions};
use meshway_protocol::*;
use std::{
    ops::{Deref, DerefMut},
    path::Path,
};

pub(crate) const LEGACY_NODE_SELECT: &str = "SELECT id,name,agent_version,registered_at,last_seen,revoked_at,mesh_ipv4,mesh_ipv6,country,(SELECT revision FROM configuration WHERE id=1),acknowledged_revision FROM nodes";

pub(crate) struct SingleNetworkFixture {
    inner: Store,
    pub network_id: NetworkId,
}
impl Deref for SingleNetworkFixture {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.inner
    }
}
impl DerefMut for SingleNetworkFixture {
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

impl SingleNetworkFixture {
    pub fn open(path: &Path, options: &NetworkOptions) -> Result<Self, Error> {
        let inner = Store::open(path, options)?;
        let network_id = inner.initial_network_id()?;
        Ok(Self { inner, network_id })
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
}

pub(crate) fn original_columns(db: &rusqlite::Connection, table: &str) -> String {
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
