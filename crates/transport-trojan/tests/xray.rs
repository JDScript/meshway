//! Docker-backed real interoperability. Run through ../interop/run.py so every
//! Docker operation is inside the repository's shared performance lock.
mod support;

use std::{
    fs::OpenOptions, io::Write, net::SocketAddr, os::unix::fs::OpenOptionsExt, path::Path,
    process::Command, time::Duration,
};

use meshway_transport_trojan::{SecretPassword, Target, tls::ClientTls};
use serde_json::json;
use support::{Certificates, SERVER_NAME, configs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use zeroize::Zeroizing;

const IMAGE: &str = "ghcr.io/xtls/xray-core@sha256:592ec4d11f656db95598d01e76dbcc6e002d67360b96a5436500a938230f52c7";
const DEADLINE: Duration = Duration::from_secs(15);

fn docker(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("Docker invocation failed");
    // Do not print config, logs, credentials, or full command output on failure.
    assert!(
        output.status.success(),
        "Docker command failed (exit {:?})",
        output.status.code()
    );
    String::from_utf8(output.stdout).expect("non-UTF8 Docker output")
}

fn private_file(path: &Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

fn published_port(container: &str, port: &str) -> SocketAddr {
    docker(&["port", container, port]).trim().parse().unwrap()
}

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker and the shared performance lock; use interop/run.py"]
async fn verified_tls_xray_both_directions() {
    let container_name = std::env::var("MESHWAY_TROJAN_INTEROP_CONTAINER")
        .expect("run interop/run.py to acquire performance_lock first");
    assert!(container_name.starts_with("meshway-trojan-interop-"));
    let temporary_root = std::env::var("MESHWAY_TROJAN_TEMP_ROOT").unwrap();
    let files = tempfile::Builder::new()
        .prefix("trojan-xray-")
        .tempdir_in(temporary_root)
        .unwrap();
    let certs = Certificates::new(false);
    private_file(
        &files.path().join("server.pem"),
        certs.leaf.pem().as_bytes(),
    );
    let pem_key = Zeroizing::new(certs.key.serialize_pem());
    private_file(&files.path().join("server.key"), pem_key.as_bytes());

    let mut random = Zeroizing::new([0; 32]);
    getrandom::fill(&mut random[..]).unwrap();
    let password = Zeroizing::new(
        random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    );
    let (mut client_config, mut server_config) = configs();
    client_config.password = SecretPassword::new(password.as_bytes()).unwrap();
    server_config.password = client_config.password.clone();

    // Docker Desktop reaches these private host fixtures through host.docker.internal.
    let echo = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let ours = TcpListener::bind("0.0.0.0:0").await.unwrap();
    client_config.target =
        Target::domain("host.docker.internal", echo.local_addr().unwrap().port()).unwrap();
    server_config.expected_target = Target::ip("127.0.0.1".parse().unwrap(), 4242).unwrap();
    let config = json!({
        "log": {"loglevel": "none"},
        "inbounds": [
            {"tag": "trojan-in", "listen": "0.0.0.0", "port": 24443, "protocol": "trojan",
             "settings": {"clients": [{"password": password.as_str()}]},
             "streamSettings": {"network": "tcp", "security": "tls", "tlsSettings": {
                 "certificates": [{"certificateFile": "/fixture/server.pem", "keyFile": "/fixture/server.key"}]
             }}},
            {"tag": "native-in", "listen": "0.0.0.0", "port": 24444, "protocol": "socks",
             "settings": {"auth": "noauth", "udp": false}}
        ],
        "outbounds": [
            {"tag": "direct", "protocol": "freedom"},
            {"tag": "to-meshway", "protocol": "trojan",
             "settings": {"servers": [{"address": "host.docker.internal", "port": ours.local_addr().unwrap().port(), "password": password.as_str()}]},
             "streamSettings": {"network": "tcp", "security": "tls", "tlsSettings": {
                 "serverName": SERVER_NAME, "allowInsecure": false, "disableSystemRoot": true,
                 "certificates": [{"usage": "verify", "certificate": certs.ca.pem().lines().collect::<Vec<_>>()}]
             }}}
        ],
        "routing": {"rules": [{"type": "field", "inboundTag": ["native-in"], "outboundTag": "to-meshway"}]}
    });
    let config_bytes = Zeroizing::new(serde_json::to_vec(&config).unwrap());
    private_file(&files.path().join("config.json"), &config_bytes);
    drop(config);
    drop(password);
    drop(random);
    drop(pem_key);
    let volume = format!("{}:/fixture:ro", files.path().display());
    let container = Container(container_name);
    docker(&[
        "run",
        "-d",
        "--name",
        &container.0,
        "--label",
        "io.meshway.experiment=trojan-interop",
        "--label",
        &format!("io.meshway.run={}", container.0),
        "-p",
        "127.0.0.1::24443",
        "-p",
        "127.0.0.1::24444",
        "-v",
        &volume,
        "--entrypoint",
        "/usr/local/bin/xray",
        IMAGE,
        "run",
        "-config",
        "/fixture/config.json",
    ]);
    let version = docker(&["exec", &container.0, "/usr/local/bin/xray", "version"]);
    println!("upstream: {}", version.lines().next().unwrap());
    let upstream = published_port(&container.0, "24443/tcp");
    let native = published_port(&container.0, "24444/tcp");
    timeout(DEADLINE, async {
        loop {
            if TcpStream::connect(upstream).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();

    let payload = (0..65536).map(|i| (i % 256) as u8).collect::<Vec<_>>();
    let client_tls = certs.client();
    // Our real TLS+Trojan client -> Xray Trojan inbound -> private binary echo.
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                let mut stream = client_tls
                    .dial(upstream, &client_config, DEADLINE)
                    .await
                    .unwrap();
                // Client-first payload (the same ordering required by Noise IK).
                stream.write_all(&payload).await.unwrap();
                stream.flush().await.unwrap();
                let mut response = vec![0; payload.len() + 4];
                stream.read_exact(&mut response).await.unwrap();
                assert_eq!(&response[..4], b"\0\xff\x80\x01");
                assert_eq!(&response[4..], payload);
                stream.shutdown().await.unwrap();
            },
            async {
                let (mut stream, _) = echo.accept().await.unwrap();
                stream.write_all(b"\0\xff\x80\x01").await.unwrap();
                let mut incoming = vec![0; payload.len()];
                stream.read_exact(&mut incoming).await.unwrap();
                assert_eq!(incoming, payload);
                stream.write_all(&incoming).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
    println!("PASS Meshway client -> Xray server: verified private CA + binary roundtrip");

    let server_tls = certs.server();
    // Native Xray Trojan outbound -> our fixed-target TLS server.
    timeout(DEADLINE, async {
        tokio::join!(
            async {
                let mut stream = TcpStream::connect(native).await.unwrap();
                // Drive the native Trojan outbound through Xray's local SOCKS
                // inbound. This fixture port is published only on host loopback.
                stream.write_all(&[5, 1, 0]).await.unwrap();
                let mut method = [0; 2];
                stream.read_exact(&mut method).await.unwrap();
                assert_eq!(method, [5, 0]);
                stream
                    .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, 0x10, 0x92])
                    .await
                    .unwrap();
                let mut response = [0; 4];
                stream.read_exact(&mut response).await.unwrap();
                assert_eq!(&response[..3], &[5, 0, 0]);
                match response[3] {
                    1 => {
                        stream.read_exact(&mut [0; 6]).await.unwrap();
                    }
                    4 => {
                        stream.read_exact(&mut [0; 18]).await.unwrap();
                    }
                    _ => panic!("unexpected SOCKS reply address type"),
                }
                stream.write_all(&payload[..1]).await.unwrap();
                let mut greeting = [0; 4];
                stream.read_exact(&mut greeting).await.unwrap();
                assert_eq!(&greeting, b"\xff\0\x01\x80");
                stream.write_all(&payload[1..]).await.unwrap();
                let mut response = vec![0; payload.len()];
                stream.read_exact(&mut response).await.unwrap();
                assert_eq!(response, payload);
                stream.shutdown().await.unwrap();
            },
            async {
                let (stream, _) = ours.accept().await.unwrap();
                let (mut stream, request) = server_tls
                    .accept(stream, &server_config, DEADLINE)
                    .await
                    .unwrap();
                assert_eq!(request.target, server_config.expected_target);
                stream.write_all(b"\xff\0\x01\x80").await.unwrap();
                stream.flush().await.unwrap();
                let mut incoming = vec![0; payload.len()];
                stream.read_exact(&mut incoming).await.unwrap();
                assert_eq!(incoming, payload);
                stream.write_all(&incoming).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        );
    })
    .await
    .unwrap();
    println!(
        "PASS Xray client -> Meshway server: verified private CA + fixed target + binary roundtrip"
    );

    let wrong_name = ClientTls::new("wrong.example", certs.roots()).unwrap();
    assert!(
        wrong_name
            .dial(upstream, &client_config, DEADLINE)
            .await
            .is_err()
    );
    client_config.password = SecretPassword::new(b"deliberately wrong password").unwrap();
    timeout(DEADLINE, async {
        let mut stream = client_tls
            .dial(upstream, &client_config, DEADLINE)
            .await
            .unwrap();
        // Client handshake can't report Trojan acceptance: no wire ACK exists.
        let _ = stream.write_all(b"\0\xffbad auth").await;
        let _ = stream.flush().await;
        let result = stream.read(&mut [0; 1]).await;
        assert!(matches!(result, Ok(0) | Err(_)));
    })
    .await
    .unwrap();
    println!("PASS upstream negative checks: wrong TLS server name and wrong Trojan password");
}
