/// Bounded error categories. Neither formatting nor error sources expose
/// certificates, private keys, server names or peer-controlled error messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("TLS certificate material is missing, malformed or exceeds its limit")]
    Certificate,
    #[error("TLS private key is missing, unsupported, malformed or mismatched")]
    PrivateKey,
    #[error("invalid TLS server name")]
    ServerName,
    #[error("TLS certificate chain, name, signature or validity verification failed")]
    CertificateVerification,
    #[error("unsupported TLS version range")]
    VersionRange,
    #[error("invalid or oversized TLS ALPN configuration")]
    Alpn,
    #[error("TLS handshake timeout is not positive and representable")]
    TimeoutRange,
    #[error("unsupported TLS configuration")]
    Configuration,
    #[error("TLS handshake failed")]
    Handshake,
    #[error("TLS handshake timed out")]
    Timeout,
}
