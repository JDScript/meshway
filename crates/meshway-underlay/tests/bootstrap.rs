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
    let mut host = Host::open(directory, &origin, options).await.unwrap();
    let mut builder = host
        .client_builder()
        .timeout(std::time::Duration::from_secs(10));
    if let Ok(path) = std::env::var("MESHWAY_UNDERLAY_CA") {
        let certificate = reqwest::Certificate::from_pem(&std::fs::read(path).unwrap()).unwrap();
        builder = builder.tls_certs_merge([certificate]);
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
