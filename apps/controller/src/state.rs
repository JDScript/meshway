//! Application lifetime and the bounded blocking storage entry point.
use crate::{Error, accounts, network_scope::NetworkId, store::Store};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct AppState {
    store: Arc<Mutex<Store>>,
    accounts: Arc<accounts::Ingress>,
}

#[derive(Clone, Debug, Default)]
pub struct NetworkOptions {
    pub ipv4_prefix: Option<String>,
    pub ipv6_prefix: Option<String>,
}

impl AppState {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open_with_network(data_dir, NetworkOptions::default())
    }
    pub fn open_with_network(
        data_dir: impl AsRef<Path>,
        options: NetworkOptions,
    ) -> Result<Self, Error> {
        let store = Store::open(data_dir.as_ref(), &options)?;
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            accounts: Arc::new(accounts::Ingress::new()?),
        })
    }
    pub(crate) fn accounts(&self) -> &accounts::Ingress {
        &self.accounts
    }

    pub(crate) async fn access<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Store) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || action(&mut *store.lock().map_err(|_| Error::Worker)?))
            .await
            .map_err(|_| Error::Worker)?
    }
    pub(crate) async fn network_access<T: Send + 'static>(
        &self,
        token: String,
        network_id: String,
        permission: accounts::NetworkPermission,
        action: impl FnOnce(&mut Store, &NetworkId) -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        self.access(move |store| store.network_operation(&token, &network_id, permission, action))
            .await
    }
}
