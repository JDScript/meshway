use std::{io::Cursor, sync::Arc};

use meshway_transport::spec::ListenerTls;
use rustls::{
    RootCertStore,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    sign::CertifiedKey,
};

use crate::Error;

const MAX_CERTIFICATE_PEM: usize = 128 * 1024;
const MAX_PRIVATE_KEY_PEM: usize = 32 * 1024;
const MAX_CERTIFICATES: usize = 8;

fn certificates(pem: &str) -> Result<Vec<CertificateDer<'static>>, Error> {
    let mut result = Vec::new();
    let mut start = None;
    let mut offset = 0;
    for raw in pem.split_inclusive('\n') {
        let line = raw.trim_ascii();
        match start {
            None if line.is_empty() => {}
            None if line == "-----BEGIN CERTIFICATE-----" => {
                if result.len() == MAX_CERTIFICATES {
                    return Err(Error::Certificate);
                }
                start = Some(offset);
            }
            Some(begin) if line == "-----END CERTIFICATE-----" => {
                // Check armor shape ourselves because rustls-pemfile permits
                // skipped text/nested sections. Base64 and DER remain handled
                // by the established PEM/WebPKI implementations.
                let block = pem.as_bytes()[begin..offset + raw.len()].trim_ascii();
                let (item, rest) = rustls_pemfile::read_one_from_slice(block)
                    .map_err(|_| Error::Certificate)?
                    .ok_or(Error::Certificate)?;
                let rustls_pemfile::Item::X509Certificate(certificate) = item else {
                    return Err(Error::Certificate);
                };
                if !rest.trim_ascii().is_empty() {
                    return Err(Error::Certificate);
                }
                result.push(certificate);
                start = None;
            }
            Some(_)
                if line.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || byte.is_ascii_whitespace()
                        || b"+/=".contains(&byte)
                }) => {}
            _ => return Err(Error::Certificate),
        }
        offset += raw.len();
    }
    if start.is_some() || result.is_empty() {
        return Err(Error::Certificate);
    }
    Ok(result)
}

pub(crate) fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub(crate) struct Chain {
    pub(crate) certificates: Vec<CertificateDer<'static>>,
    pub(crate) name: ServerName<'static>,
    pub(crate) verifier: Arc<WebPkiServerVerifier>,
}

impl Chain {
    pub(crate) fn parse(
        pem: &str,
        name: &str,
        provider: Arc<CryptoProvider>,
    ) -> Result<Self, Error> {
        if pem.len() > MAX_CERTIFICATE_PEM {
            return Err(Error::Certificate);
        }
        let certificates = certificates(pem)?;
        if certificates.is_empty() || certificates.len() > MAX_CERTIFICATES {
            return Err(Error::Certificate);
        }
        let mut roots = RootCertStore::empty();
        // The supplied chain's final certificate is the only trust anchor,
        // matching the Server material verifier (including self-signed leaves).
        roots
            .add(certificates.last().ok_or(Error::Certificate)?.clone())
            .map_err(|_| Error::Certificate)?;
        let verifier = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|_| Error::Certificate)?;
        let name = ServerName::try_from(name.to_owned()).map_err(|_| Error::ServerName)?;
        Ok(Self {
            certificates,
            name,
            verifier,
        })
    }

    pub(crate) fn verify(&self) -> Result<(), Error> {
        self.verifier
            .verify_server_cert(
                &self.certificates[0],
                &self.certificates[1..],
                &self.name,
                &[],
                UnixTime::now(),
            )
            .map_err(|_| Error::CertificateVerification)?;
        Ok(())
    }
}

/// Shared implementation for the public material helper and the TLS server.
/// Retaining the parsed signing key lets the server cache it without reparsing.
pub(crate) fn listener(
    tls: &ListenerTls,
    provider: Arc<CryptoProvider>,
) -> Result<CertifiedKey, Error> {
    let pem = tls.certificate.as_ref().ok_or(Error::Certificate)?.expose();
    let key = tls.private_key.as_ref().ok_or(Error::PrivateKey)?.expose();
    if key.len() > MAX_PRIVATE_KEY_PEM {
        return Err(Error::PrivateKey);
    }
    let chain = Chain::parse(pem, &tls.server_name, provider.clone())?;
    let private_key = rustls_pemfile::private_key(&mut Cursor::new(key))
        .map_err(|_| Error::PrivateKey)?
        .ok_or(Error::PrivateKey)?;
    let certified = CertifiedKey::from_der(chain.certificates.clone(), private_key, &provider)
        .map_err(|_| Error::PrivateKey)?;
    certified.keys_match().map_err(|_| Error::PrivateKey)?;
    chain.verify()?;
    Ok(certified)
}

/// Verify certificate chain, DNS/IP name, current validity and signing-key
/// consistency using explicit ring/WebPKI, returning the exact leaf DER.
///
/// Limits: 128 KiB certificate PEM, 32 KiB private-key PEM and eight certificates.
/// Only the supplied chain's final certificate is a trust anchor. This checks
/// material, not protocol options, revocation, publication or node identity.
pub fn verify_listener(tls: &ListenerTls) -> Result<Vec<u8>, Error> {
    Ok(listener(tls, provider())?.cert[0].as_ref().to_vec())
}
