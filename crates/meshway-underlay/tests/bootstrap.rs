#![cfg(target_os = "linux")]

use meshway_underlay::{Host, Options};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

/// A fixture can use either an explicit real DNS upstream or a hosts override.
/// The original HTTPS origin and its certificate name remain unchanged.
#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace and HTTPS Server fixture required"]
async fn fixed_origin_https_survives_explicit_network_down() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let origin = std::env::var("MESHWAY_UNDERLAY_ORIGIN").expect("HTTPS fixture origin");
    let url = reqwest::Url::parse(&origin).unwrap();
    assert_eq!(url.scheme(), "https");
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(
        meshway_private_storage::Directory::open(&temp.path().join("private"), "fixture.lock")
            .unwrap(),
    );
    let mut options = Options::default();
    if let Ok(ip) = std::env::var("MESHWAY_UNDERLAY_ORIGIN_IP") {
        options.bootstrap_hosts.insert(
            url.host_str().unwrap().to_owned(),
            vec![ip.parse::<IpAddr>().unwrap()],
        );
    }
    if let Ok(server) = std::env::var("MESHWAY_UNDERLAY_NAMESERVER") {
        options.bootstrap_nameservers = Some(vec![server.parse::<SocketAddr>().unwrap()]);
    }
    let mut host = Host::open(directory, options).await.unwrap();
    assert!(matches!(
        host.client_builder(),
        Err(meshway_underlay::Error::ControlUnavailable)
    ));
    host.attach_control(&origin).await.unwrap();
    assert!(matches!(
        host.attach_control(&origin).await,
        Err(meshway_underlay::Error::Conflict)
    ));
    let mut builder = host
        .client_builder()
        .unwrap()
        .timeout(std::time::Duration::from_secs(10));
    if let Ok(path) = std::env::var("MESHWAY_UNDERLAY_CA") {
        let tls = meshway_tls::Client::new(&meshway_transport::spec::ClientTls {
            server_name: url.host_str().unwrap().into(),
            certificate_chain: meshway_transport::spec::SecretString::new(
                std::fs::read_to_string(path).unwrap(),
            ),
            options: meshway_transport::spec::DialerTls {
                alpn: vec!["http/1.1".into()],
                ..Default::default()
            },
        })
        .unwrap();
        builder = builder.tls_backend_preconfigured(tls.configuration());
    }
    let client = builder.build().unwrap();
    let endpoint = url.join("api/v1/auth/options").unwrap();
    let first = client.get(endpoint.clone()).send().await.unwrap();
    assert!(first.status().is_success());
    host.down().await.unwrap();
    let second = client.get(endpoint.clone()).send().await.unwrap();
    assert!(second.status().is_success());
    host.shutdown(false).await.unwrap();
    assert!(client.get(endpoint).send().await.is_err());
}
