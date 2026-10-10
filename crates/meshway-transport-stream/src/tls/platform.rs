use std::sync::Arc;

use rustls::ClientConfig;

use crate::tls::{Error, material};

/// System-trust client configuration for HTTP adapters, using the same explicit
/// ring provider as managed TLS. Platform verification retains OS trust policy;
/// it does not trust Meshway's configured leaf certificates implicitly.
///
/// The caller owns sockets, DNS, request/handshake deadlines and HTTP settings.
/// No global provider is installed. The current HTTP clients use HTTP/1.1 only.
pub fn platform_client_config() -> Result<ClientConfig, Error> {
    let provider = material::provider();
    let verifier = rustls_platform_verifier::Verifier::new(provider.clone())
        .map_err(|_| Error::Configuration)?;
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Configuration)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}
