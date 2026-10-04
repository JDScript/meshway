use crate::{Cli, Result};
use meshway_transport_spec::{ListenerTls, SecretString, TlsMode, TlsVersion};

pub async fn prepare(cli: &Cli) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    tokio::fs::create_dir_all(&cli.public).await?;
    tokio::fs::create_dir_all(&cli.private).await?;
    let certified = rcgen::generate_simple_self_signed(vec!["meshway-packet-controller".into()])?;
    let mut key_options = tokio::fs::OpenOptions::new();
    key_options.write(true).create_new(true);
    #[cfg(unix)]
    key_options.mode(0o600);
    let mut key = key_options.open(cli.private.join("server.key")).await?;
    key.write_all(certified.signing_key.serialize_pem().as_bytes())
        .await?;
    key.sync_all().await?;
    let mut cert = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(cli.public.join("server.pem"))
        .await?;
    cert.write_all(certified.cert.pem().as_bytes()).await?;
    cert.sync_all().await?;
    tracing::info!(phase = "prepared", "isolated fixture TLS prepared");
    Ok(())
}

pub async fn client(cli: &Cli) -> Result<reqwest::ClientBuilder> {
    let pem = tokio::fs::read(cli.public.join("server.pem")).await?;
    Ok(reqwest::Client::builder()
        .no_proxy()
        .timeout(crate::IO)
        .add_root_certificate(reqwest::Certificate::from_pem(&pem)?))
}

pub async fn server(cli: &Cli) -> Result<meshway_tls::Server> {
    Ok(meshway_tls::Server::new(&ListenerTls {
        mode: TlsMode::Provided,
        server_name: "meshway-packet-controller".into(),
        certificate: Some(SecretString::new(
            tokio::fs::read_to_string(cli.public.join("server.pem")).await?,
        )),
        private_key: Some(SecretString::new(
            tokio::fs::read_to_string(cli.private.join("server.key")).await?,
        )),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: vec!["http/1.1".into()],
        handshake_timeout: "5s".parse()?,
    })?)
}
