use std::{fmt, sync::Arc, time::Duration};

use meshway_transport::spec::{ClientTls, TlsTrust};
use rustls::{
    ClientConfig, DigitallySignedStruct, SignatureScheme,
    client::{
        Resumption, WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::{TlsConnector, TlsStream};

use crate::{
    Error,
    material::{self, Chain},
    options::{self, Options},
};

/// Cached client configuration with WebPKI verification and an exact leaf pin.
#[derive(Clone)]
pub struct Client {
    connector: TlsConnector,
    name: ServerName<'static>,
    timeout: Duration,
}

impl Client {
    /// Validate supplied material and options without opening a connection.
    /// ALPN entries retain their order and are limited to 255 bytes each and
    /// 16 KiB in total (including each entry's one-byte length).
    pub fn new(tls: &ClientTls) -> Result<Self, Error> {
        let TlsTrust::Managed = tls.options.trust;
        let options = Options::new(
            tls.options.min_version,
            tls.options.max_version,
            &tls.options.alpn,
            &tls.options.handshake_timeout,
        )?;
        let provider = material::provider();
        let chain = Chain::parse(
            tls.certificate_chain.expose(),
            &tls.server_name,
            provider.clone(),
        )?;
        chain.verify()?;
        let verifier = Arc::new(ExactLeaf {
            webpki: chain.verifier,
            leaf: chain.certificates[0].clone(),
        });
        let mut config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(options.versions)
            .map_err(|_| Error::Configuration)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        config.alpn_protocols = options.alpn;
        // Resumption skips certificate verification; every connection must
        // recheck the exact leaf and its validity at handshake time.
        config.resumption = Resumption::disabled();
        Ok(Self {
            connector: TlsConnector::from(Arc::new(config)),
            name: chain.name,
            timeout: options.timeout,
        })
    }

    /// Clone the same exact-leaf configuration for an HTTP or other TLS adapter.
    /// The adapter owns handshake timeouts; resumption stays disabled so each
    /// new connection checks the leaf, name and current certificate validity.
    pub fn configuration(&self) -> ClientConfig {
        self.connector.config().as_ref().clone()
    }

    /// Negotiate TLS over an already-open stream. The caller's overall timeout or
    /// remaining lease must enclose this handshake's independent local cap.
    pub async fn connect<S>(&self, stream: S) -> Result<TlsStream<S>, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        tokio::time::timeout_at(
            options::deadline(self.timeout)?,
            self.connector.connect(self.name.clone(), stream),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map(Into::into)
        .map_err(|_| Error::Handshake)
    }
}

struct ExactLeaf {
    webpki: Arc<WebPkiServerVerifier>,
    leaf: CertificateDer<'static>,
}

impl fmt::Debug for ExactLeaf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ExactLeaf")
    }
}

impl ServerCertVerifier for ExactLeaf {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let verified = self.webpki.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )?;
        if end_entity.as_ref() != self.leaf.as_ref() {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(verified)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.webpki.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.webpki.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exact_leaf_still_requires_the_handshake_name_and_current_validity() {
        let mut params = rcgen::CertificateParams::new(vec!["edge.example.test".into()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2020, 1, 1);
        params.not_after = rcgen::date_time_ymd(2030, 1, 1);
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = params.self_signed(&key).unwrap();
        let chain = Chain::parse(
            &certificate.pem(),
            "edge.example.test",
            material::provider(),
        )
        .unwrap();
        let verifier = ExactLeaf {
            webpki: chain.verifier,
            leaf: chain.certificates[0].clone(),
        };
        let at_year = |year| {
            UnixTime::since_unix_epoch(Duration::from_secs(
                u64::try_from(rcgen::date_time_ymd(year, 1, 1).unix_timestamp()).unwrap(),
            ))
        };
        verifier
            .verify_server_cert(certificate.der(), &[], &chain.name, &[], at_year(2026))
            .unwrap();
        let wrong_name = ServerName::try_from("other.example.test").unwrap();
        assert!(
            verifier
                .verify_server_cert(certificate.der(), &[], &wrong_name, &[], at_year(2026))
                .is_err()
        );
        for year in [2019, 2031] {
            assert!(
                verifier
                    .verify_server_cert(certificate.der(), &[], &chain.name, &[], at_year(year))
                    .is_err()
            );
        }
    }
}
