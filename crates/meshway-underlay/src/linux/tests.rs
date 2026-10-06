use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn kernel_only_host_defers_control_and_revokes_retained_factory() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("private"), "test.lock").unwrap());
    // This resolver configuration is intentionally unusable. Opening the kernel
    // and protected factory must not open a resolver or require a Server origin.
    let options = Options {
        bootstrap_nameservers: Some(vec!["127.0.0.1:53".parse().unwrap()]),
        ..Default::default()
    };
    let mut host = Host::open(directory.clone(), options.clone())
        .await
        .unwrap();
    let sockets = host.sockets();
    assert!(matches!(
        host.client_builder(),
        Err(Error::ControlUnavailable)
    ));
    assert!(!directory.path().join("underlay-bootstrap.sock").exists());
    assert!(Host::open(directory.clone(), options).await.is_err());
    let other = Arc::new(Directory::open(&temp.path().join("other"), "test.lock").unwrap());
    assert!(matches!(
        Host::open(other, Options::default()).await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        host.attach_control("invalid").await,
        Err(Error::Origin)
    ));
    assert!(matches!(
        host.attach_control("https://example.test").await,
        Err(Error::Resolver)
    ));
    assert!(matches!(
        host.client_builder(),
        Err(Error::ControlUnavailable)
    ));
    sockets.shared.check().unwrap();
    host.shutdown(false).await.unwrap();
    assert!(matches!(sockets.shared.check(), Err(Error::Worker)));
    assert!(matches!(sockets.resolver().await, Err(Error::Worker)));

    // A retained, revoked factory neither keeps a second owner alive nor obtains
    // fresh authority when the private directory is opened again.
    let host = Host::open(directory, Options::default()).await.unwrap();
    assert!(matches!(sockets.shared.check(), Err(Error::Worker)));
    let current = host.sockets();
    drop(host);
    assert!(matches!(current.shared.check(), Err(Error::Worker)));
}

#[tokio::test]
#[ignore = "isolated Linux NET_ADMIN namespace required"]
async fn control_attachment_relays_before_and_after_down_until_shutdown() {
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = Arc::new(Directory::open(&temp.path().join("private"), "test.lock").unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
    });
    let mut host = Host::open(directory.clone(), Options::default())
        .await
        .unwrap();
    host.attach_control(&origin).await.unwrap();
    assert!(matches!(
        host.attach_control(&origin).await,
        Err(Error::Conflict)
    ));
    let client = host
        .client_builder()
        .unwrap()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(&origin)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    host.down().await.unwrap();
    assert_eq!(
        client
            .get(&origin)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    server.await.unwrap();
    host.shutdown(false).await.unwrap();
    assert!(!directory.path().join("underlay-bootstrap.sock").exists());
    assert!(client.get(&origin).send().await.is_err());
}
