use std::{net::SocketAddr, time::Duration};

use meshway_control_client::http_client_builder;
use meshway_transport::spec::{
    ClientTls, DialerTls, ListenerTls, SecretString, TlsMode, TlsVersion,
};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const NAME: &str = "control.example.test";

fn authority() -> CertifiedIssuer<'static, KeyPair> {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap()
}

fn material(authority: &CertifiedIssuer<'_, KeyPair>, version: TlsVersion) -> ListenerTls {
    let key = KeyPair::generate().unwrap();
    let cert = CertificateParams::new(vec![NAME.into()])
        .unwrap()
        .signed_by(&key, authority)
        .unwrap();
    ListenerTls {
        mode: TlsMode::Provided,
        server_name: NAME.into(),
        certificate: Some(SecretString::new(cert.pem() + &authority.pem())),
        private_key: Some(SecretString::new(key.serialize_pem())),
        min_version: version,
        max_version: version,
        alpn: vec!["http/1.1".into()],
        handshake_timeout: "2s".parse().unwrap(),
    }
}

async fn serve(tls: &ListenerTls) -> (SocketAddr, tokio::task::JoinHandle<bool>) {
    let server = meshway_transport_stream::tls::Server::new(tls).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let Ok(mut stream) = server.accept(stream).await else {
            return false;
        };
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if headers.len() == 4096 || stream.read_exact(&mut byte).await.is_err() {
                return false;
            }
            headers.push(byte[0]);
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        true
    });
    (address, task)
}

#[tokio::test]
async fn preconfigured_http_checks_name_and_exact_leaf_for_both_tls_versions() {
    let ca = authority();
    for version in [TlsVersion::Tls12, TlsVersion::Tls13] {
        let expected = material(&ca, version);
        let replacement = material(&ca, version);
        let config = meshway_transport_stream::tls::Client::new(&ClientTls {
            server_name: NAME.into(),
            certificate_chain: expected.certificate.clone().unwrap(),
            options: DialerTls {
                min_version: version,
                max_version: version,
                alpn: vec!["http/1.1".into()],
                ..Default::default()
            },
        })
        .unwrap()
        .configuration();
        for (served, name, accepted) in [
            (&expected, NAME, true),
            (&replacement, NAME, false),
            (&expected, "wrong.example.test", false),
        ] {
            let (address, task) = serve(served).await;
            let client = reqwest::Client::builder()
                .tls_backend_preconfigured(config.clone())
                .no_proxy()
                .resolve(name, address)
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let response = client
                .get(format!("https://{name}:{}/", address.port()))
                .send()
                .await;
            assert_eq!(response.is_ok(), accepted);
            if let Ok(response) = response {
                assert_eq!(response.text().await.unwrap(), "ok");
            }
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap(),
                accepted
            );
        }
    }
}

#[tokio::test]
async fn system_http_rejects_private_ca_without_installing_a_global_provider() {
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
    let tls = material(&authority(), TlsVersion::Tls13);
    let (address, task) = serve(&tls).await;
    let client = http_client_builder()
        .unwrap()
        .no_proxy()
        .resolve(NAME, address)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert!(
        client
            .get(format!("https://{NAME}:{}/", address.port()))
            .send()
            .await
            .is_err()
    );
    assert!(
        !tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
    );
    assert!(rustls::crypto::CryptoProvider::get_default().is_none());
}
