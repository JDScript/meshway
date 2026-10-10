use crate::{Cli, Result};

pub async fn prepare(cli: &Cli) -> Result<()> {
    meshway_packet_fixture::prepare_tls(&cli.public, &cli.private).await?;
    tracing::info!(phase = "prepared", "isolated fixture TLS prepared");
    Ok(())
}

pub async fn client(cli: &Cli) -> Result<reqwest::ClientBuilder> {
    meshway_packet_fixture::tls_client(&cli.public, crate::IO).await
}

pub async fn server(cli: &Cli) -> Result<meshway_transport_stream::tls::Server> {
    meshway_packet_fixture::tls_server(&cli.public, &cli.private).await
}
