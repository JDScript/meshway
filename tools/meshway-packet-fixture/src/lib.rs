//! Shared HTTPS material and rendezvous for packet-library and real-daemon conformance.
#![forbid(unsafe_code)]

pub mod mtu;
pub mod rendezvous;

use meshway_transport::spec::{
    ClientTls, DialerTls, ListenerTls, SecretString, TlsMode, TlsVersion,
};
use std::{path::Path, time::Duration};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub const CONTROLLER_NAME: &str = "meshway-packet-controller";
pub const CONTROLLER_ORIGIN: &str = "https://meshway-packet-controller:8443/";

pub async fn prepare_tls(public: &Path, private: &Path) -> Result<(), Error> {
    use tokio::io::AsyncWriteExt;
    tokio::fs::create_dir_all(public).await?;
    tokio::fs::create_dir_all(private).await?;
    let certified = rcgen::generate_simple_self_signed(vec![CONTROLLER_NAME.into()])?;
    for (path, bytes, mode) in [
        (
            private.join("server.key"),
            certified.signing_key.serialize_pem(),
            0o600,
        ),
        (public.join("server.pem"), certified.cert.pem(), 0o644),
    ] {
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(mode);
        let mut file = options.open(path).await?;
        file.write_all(bytes.as_bytes()).await?;
        file.sync_all().await?;
    }
    Ok(())
}

pub async fn tls_client(public: &Path, timeout: Duration) -> Result<reqwest::ClientBuilder, Error> {
    let pem = tokio::fs::read_to_string(public.join("server.pem")).await?;
    let tls = meshway_transport_stream::tls::Client::new(&ClientTls {
        server_name: CONTROLLER_NAME.into(),
        certificate_chain: SecretString::new(pem),
        options: DialerTls {
            alpn: vec!["http/1.1".into()],
            ..DialerTls::default()
        },
    })?;
    Ok(reqwest::Client::builder()
        .tls_backend_preconfigured(tls.configuration())
        .no_proxy()
        .timeout(timeout))
}

pub async fn tls_server(
    public: &Path,
    private: &Path,
) -> Result<meshway_transport_stream::tls::Server, Error> {
    Ok(meshway_transport_stream::tls::Server::new(&ListenerTls {
        mode: TlsMode::Provided,
        server_name: CONTROLLER_NAME.into(),
        certificate: Some(SecretString::new(
            tokio::fs::read_to_string(public.join("server.pem")).await?,
        )),
        private_key: Some(
            SecretString::new(tokio::fs::read_to_string(private.join("server.key")).await?).into(),
        ),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: vec!["http/1.1".into()],
        handshake_timeout: "5s".parse()?,
    })?)
}
