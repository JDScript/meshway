//! The connection boundary is injectable, but URL/public-address admission,
//! timeouts, redirect rejection and byte budgets stay in the downloader.
use meshway_protocol::resources::ResourceProblem;
use std::{future::Future, net::SocketAddr, pin::Pin, time::Duration};

pub(crate) type ResolveFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<SocketAddr>, ResourceProblem>> + Send + 'a>>;
pub(crate) trait Connector: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a>;
    fn client(
        &self,
        host: &str,
        addresses: &[SocketAddr],
    ) -> Result<reqwest::Client, ResourceProblem>;
}
pub(crate) struct Https;
impl Connector for Https {
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> ResolveFuture<'a> {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host, port))
                .await
                .map_err(|_| ResourceProblem::DownloadFailed)?
                .collect())
        })
    }
    fn client(
        &self,
        host: &str,
        addresses: &[SocketAddr],
    ) -> Result<reqwest::Client, ResourceProblem> {
        client_builder(host, addresses)
            .build()
            .map_err(|_| ResourceProblem::DownloadFailed)
    }
}
pub(crate) fn client_builder(host: &str, addresses: &[SocketAddr]) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .resolve_to_addrs(host, addresses)
}

pub(crate) async fn download(
    source: &str,
    connector: &dyn Connector,
) -> Result<Vec<u8>, ResourceProblem> {
    let url = reqwest::Url::parse(source).map_err(|_| ResourceProblem::InvalidSource)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || source.len() > 2048
        || source.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ResourceProblem::InvalidSource);
    }
    let host = url.host_str().ok_or(ResourceProblem::InvalidSource)?;
    let port = url
        .port_or_known_default()
        .ok_or(ResourceProblem::InvalidSource)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        let addresses = connector.resolve(host, port).await?;
        if addresses.is_empty()
            || addresses.len() > 16
            || addresses
                .iter()
                .any(|a| !super::geoip::is_public(a.ip()) || a.port() != port)
        {
            return Err(ResourceProblem::InvalidSource);
        }
        let mut response = connector
            .client(host, &addresses)?
            .get(url.clone())
            .send()
            .await
            .map_err(|_| ResourceProblem::DownloadFailed)?;
        if !response.status().is_success() {
            return Err(ResourceProblem::DownloadFailed);
        }
        if response
            .content_length()
            .is_some_and(|size| size > super::MAX_DATASET_BYTES as u64)
        {
            return Err(ResourceProblem::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ResourceProblem::DownloadFailed)?
        {
            if bytes.len() + chunk.len() > super::MAX_DATASET_BYTES {
                return Err(ResourceProblem::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| ResourceProblem::DownloadFailed)?
}
