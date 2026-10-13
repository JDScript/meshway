//! Deployment-owned IP datasets; no configuration/entity/SSE projection types.
use crate::error::Error;
use crate::{
    state::service::Runtime,
    store::{self, Handle},
};
use chrono::Utc;
use meshway_model::ArtifactId;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpDatasetKind {
    GeoIp,
    Asn,
}
impl std::str::FromStr for IpDatasetKind {
    type Err = &'static str;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "geoip" => Ok(Self::GeoIp),
            "asn" => Ok(Self::Asn),
            _ => Err("expected geoip or asn"),
        }
    }
}
/// What is installed right now: the artifact each lookup cites as `datasetId`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpDatasetStatus {
    pub geoip: Option<ArtifactId>,
    pub asn: Option<ArtifactId>,
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
    pub fn status(&self) -> Result<IpDatasetStatus, Error> {
        let runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
        let (geoip, asn) = runtime.geoip.dataset_ids();
        Ok(IpDatasetStatus { geoip, asn })
    }
    /// A diagnostic lookup against the installed datasets; it is cached like
    /// a State derivation but creates no observation.
    pub fn lookup(&self, ip: std::net::IpAddr) -> Result<meshway_model::resources::IpInfo, Error> {
        let mut runtime = self.runtime.lock().map_err(|_| Error::RUNTIME)?;
        Ok(runtime.geoip.lookup(ip, Utc::now()))
    }
    /// Reinstalls immutable data, never volatile reports or observations.
    pub async fn restore(&self) -> Result<(), Error> {
        let runtime = self.runtime.clone();
        self.handle
            .read(move |tx| {
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                for kind in [IpDatasetKind::GeoIp, IpDatasetKind::Asn] {
                    if let Some((id, content)) = store::resources::datasets::current(tx, kind)? {
                        install(&mut runtime.geoip, kind, id, content.0)?;
                    }
                }
                Ok(())
            })
            .await
    }
    /// Deployment-only operation. Accepted SQL work owns commit and publication
    /// even when the caller stops awaiting the result.
    pub async fn install(
        &self,
        kind: IpDatasetKind,
        source: String,
        bytes: Vec<u8>,
    ) -> Result<ArtifactId, Error> {
        if source.is_empty() || source.len() > 256 || source.chars().any(char::is_control) {
            return Err(Error::InvalidRequest);
        }
        let id = ArtifactId::new(crate::accounts::random_uuid()?)
            .map_err(|_| Error::Internal("generated artifact ID"))?;
        install(&mut super::GeoIp::default(), kind, id, bytes.clone())?;
        let digest = super::content_digest(&bytes);
        let content = store::resources::datasets::MmdbContent(bytes);
        let runtime = self.runtime.clone();
        // Installation must follow successful SQL COMMIT on this same worker,
        // with the runtime guard still held even if the HTTP waiter is gone.
        self.handle
            .execute(move |connection| {
                let tx = connection.transaction()?;
                let mut runtime = runtime.lock().map_err(|_| Error::RUNTIME)?;
                store::resources::datasets::insert(
                    &tx,
                    kind,
                    &source,
                    id,
                    &content,
                    &digest,
                    Utc::now(),
                )?;
                tx.commit()?;
                install(&mut runtime.geoip, kind, id, content.0)?;
                Ok(id)
            })
            .await?
    }
}
fn install(
    geoip: &mut super::GeoIp,
    kind: IpDatasetKind,
    id: ArtifactId,
    bytes: Vec<u8>,
) -> Result<(), Error> {
    match kind {
        IpDatasetKind::GeoIp => geoip.install_city(id, bytes),
        IpDatasetKind::Asn => geoip.install_asn(id, bytes),
    }
}
