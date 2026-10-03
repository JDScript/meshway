//! Deployment-owned IP datasets; no configuration/entity/SSE projection types.
use crate::{
    state::{StateError, service::Runtime},
    store::Handle,
};
use chrono::Utc;
use meshway_model::ArtifactId;
use rusqlite::{OptionalExtension, params};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub enum IpDatasetKind {
    GeoIp,
    Asn,
}
impl IpDatasetKind {
    fn key(self) -> &'static str {
        match self {
            Self::GeoIp => "geoip",
            Self::Asn => "asn",
        }
    }
}
#[derive(Clone)]
pub struct IpDatasets {
    handle: Handle,
    runtime: Arc<Mutex<Runtime>>,
}
impl IpDatasets {
    pub(crate) fn new(handle: Handle, runtime: Arc<Mutex<Runtime>>) -> Self {
        Self { handle, runtime }
    }
    /// Reinstalls immutable data, never volatile reports or observations.
    pub async fn restore(&self) -> Result<(), StateError> {
        let runtime = self.runtime.clone();
        self.handle.execute(move |connection| {
            let tx = connection.transaction()?;
            let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
            for kind in [IpDatasetKind::GeoIp, IpDatasetKind::Asn] {
                let row: Option<(String, Vec<u8>, String)> = tx.query_row(
                    "SELECT id,data,digest FROM dataset_artifacts WHERE network_id IS NULL AND kind=?1 AND format='mmdb' ORDER BY created_at DESC,id DESC LIMIT 1",
                    [kind.key()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
                if let Some((id, bytes, digest)) = row {
                    if super::content_digest(&bytes).to_string() != digest { return Err(StateError::Storage); }
                    let id = id.parse().map_err(|_| StateError::Storage)?;
                    install(&mut runtime.geoip, kind, id, bytes)?;
                }
            }
            runtime.hub.dataset_changed();
            Ok(())
        }).await?
    }
    /// Deployment-only operation. Accepted SQL work owns commit and publication
    /// even when the caller stops awaiting the result.
    pub async fn install(
        &self,
        kind: IpDatasetKind,
        source: String,
        bytes: Vec<u8>,
    ) -> Result<ArtifactId, StateError> {
        if source.is_empty() || source.len() > 256 || source.chars().any(char::is_control) {
            return Err(StateError::Invalid);
        }
        let id =
            ArtifactId::new(crate::accounts::random_uuid().map_err(|_| StateError::Unavailable)?)
                .map_err(|_| StateError::Unavailable)?;
        install(&mut super::GeoIp::default(), kind, id, bytes.clone())?;
        let digest = super::content_digest(&bytes);
        let runtime = self.runtime.clone();
        self.handle.execute(move |connection| {
            let tx = connection.transaction()?;
            let mut runtime = runtime.lock().map_err(|_| StateError::Unavailable)?;
            tx.execute("INSERT INTO dataset_artifacts(id,network_id,kind,source,format,data,digest,created_at) VALUES (?1,NULL,?2,?3,'mmdb',?4,?5,?6)",
                params![id.to_string(), kind.key(), source, bytes, digest.to_string(), Utc::now().timestamp_millis()])?;
            tx.commit()?;
            install(&mut runtime.geoip, kind, id, bytes)?;
            runtime.hub.dataset_changed();
            Ok(id)
        }).await?
    }
}
fn install(
    geoip: &mut super::GeoIp,
    kind: IpDatasetKind,
    id: ArtifactId,
    bytes: Vec<u8>,
) -> Result<(), StateError> {
    match kind {
        IpDatasetKind::GeoIp => geoip.install_city(id, bytes),
        IpDatasetKind::Asn => geoip.install_asn(id, bytes),
    }
}
