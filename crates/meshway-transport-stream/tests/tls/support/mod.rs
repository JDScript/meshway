use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use meshway_transport::spec::{
    ClientTls, DialerTls, ListenerTls, SecretString, TlsMode, TlsVersion,
};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair,
    KeyUsagePurpose,
};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

pub const NAME: &str = "edge.example.test";

pub fn listener(certificate: &Certificate, key: &KeyPair) -> ListenerTls {
    ListenerTls {
        mode: TlsMode::Provided,
        server_name: NAME.into(),
        certificate: Some(SecretString::new(certificate.pem())),
        private_key: Some(SecretString::new(key.serialize_pem()).into()),
        min_version: TlsVersion::Tls12,
        max_version: TlsVersion::Tls13,
        alpn: vec![],
        handshake_timeout: "2s".parse().unwrap(),
    }
}

pub fn self_signed() -> ListenerTls {
    let params = CertificateParams::new(vec![NAME.into()]).unwrap();
    let key = KeyPair::generate().unwrap();
    listener(&params.self_signed(&key).unwrap(), &key)
}

pub fn authority() -> CertifiedIssuer<'static, KeyPair> {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params
        .distinguished_name
        .push(DnType::CommonName, "test CA");
    CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap()
}

pub fn issued(authority: &CertifiedIssuer<'_, KeyPair>, name: &str) -> ListenerTls {
    let params = CertificateParams::new(vec![name.into()]).unwrap();
    let key = KeyPair::generate().unwrap();
    let certificate = params.signed_by(&key, authority).unwrap();
    let mut tls = listener(&certificate, &key);
    tls.server_name = name.into();
    tls.certificate = Some(SecretString::new(certificate.pem() + &authority.pem()));
    tls
}

pub fn client(listener: &ListenerTls) -> ClientTls {
    ClientTls {
        server_name: listener.server_name.clone(),
        certificate_chain: listener.certificate.clone().unwrap(),
        options: DialerTls {
            min_version: listener.min_version,
            max_version: listener.max_version,
            alpn: listener.alpn.clone(),
            handshake_timeout: listener.handshake_timeout.clone(),
            ..DialerTls::default()
        },
    }
}

/// A real duplex endpoint whose ownership can be observed without background
/// tasks, unsafe pin projections or production-only visibility for tests.
pub struct TrackedStream {
    inner: DuplexStream,
    dropped: Arc<AtomicBool>,
}

impl TrackedStream {
    pub fn new(inner: DuplexStream) -> (Self, Arc<AtomicBool>) {
        let dropped = Arc::new(AtomicBool::new(false));
        (
            Self {
                inner,
                dropped: dropped.clone(),
            },
            dropped,
        )
    }
}

impl Drop for TrackedStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl AsyncRead for TrackedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TrackedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
