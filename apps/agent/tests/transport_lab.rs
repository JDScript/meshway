#![cfg(feature = "transport-lab")]

use std::{
    fs,
    net::UdpSocket,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn agent() -> Command {
    Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
}

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(directory: &Path, name: &str) -> Service {
    Service(
        agent()
            .args(["transport-lab", "run", "--config"])
            .arg(directory.join(format!("{name}.json")))
            .arg("--status-file")
            .arg(directory.join(format!("{name}-status.json")))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

fn run_probe(directory: &Path, node: &str, path: &str) -> std::process::Output {
    let mut child = agent()
        .args(["transport-lab", "probe", "--config"])
        .arg(directory.join(format!("{node}.json")))
        .args(["--path", path, "--bytes", "1280", "--count", "2"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lab probe exceeded its process deadline");
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn real_processes_forward_over_reverse_dialed_vless_and_stop_at_missing_relay() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = &temporary.path().join("lab");
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let stream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let output = agent()
        .args(["transport-lab", "fixture", "--directory"])
        .arg(directory)
        .arg("--lax-nyc")
        .arg(udp.local_addr().unwrap().to_string())
        .arg("--lax-sha")
        .arg(stream.local_addr().unwrap().to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    drop((udp, stream));
    let mut relay = start(directory, "lax");
    let sha = start(directory, "sha");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(raw) = fs::read_to_string(directory.join("sha-status.json")) {
            #[derive(serde::Deserialize)]
            struct Status {
                authenticated_peers: Vec<String>,
            }
            if let Ok(status) = serde_json::from_str::<Status>(&raw)
                && status.authenticated_peers.contains(&"lax".into())
            {
                break;
            }
        }
        assert!(
            relay.0.try_wait().unwrap().is_none(),
            "relay exited during startup"
        );
        assert!(Instant::now() < deadline, "SHA never authenticated to LAX");
        thread::sleep(Duration::from_millis(25));
    }
    let output = run_probe(directory, "nyc", "nyc,lax,sha");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("endpoint_authenticated_echo"));
    assert!(stdout.contains("1280"));
    assert!(!stdout.contains("private_key"));
    assert!(!stdout.contains("credential"));

    drop(sha);
    let _nyc = start(directory, "nyc");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !directory.join("nyc-status.json").exists() {
        assert!(Instant::now() < deadline, "NYC did not start");
        thread::sleep(Duration::from_millis(25));
    }
    // Probe retries below are not used: explicitly wait for the source's real
    // authenticated adjacency before originating the reverse-direction probe.
    loop {
        #[derive(serde::Deserialize)]
        struct Status {
            authenticated_peers: Vec<String>,
        }
        if let Ok(status) =
            serde_json::from_slice::<Status>(&fs::read(directory.join("nyc-status.json")).unwrap())
            && status.authenticated_peers.contains(&"lax".into())
        {
            break;
        }
        assert!(Instant::now() < deadline, "NYC did not authenticate");
        thread::sleep(Duration::from_millis(25));
    }
    let output = run_probe(directory, "sha", "sha,lax,nyc");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    drop(relay);
    let output = run_probe(directory, "sha", "sha,lax,nyc");
    assert!(
        !output.status.success(),
        "missing required relay must fail closed"
    );
}

#[test]
fn private_config_errors_do_not_echo_input_and_fixture_refuses_replacement() {
    use meshway_protocol::local;
    let temporary = tempfile::tempdir().unwrap();
    let directory = &temporary.path().join("lab");
    local::private_dir(directory).unwrap();
    let secret = "private-key-like-text-must-never-appear-in-errors";
    let path = directory.join("bad.json");
    local::atomic_write(&path, format!(r#"{{"{secret}":true}}"#).as_bytes(), false).unwrap();
    let output = agent()
        .args(["transport-lab", "check", "--config"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    let run = || {
        agent()
            .args(["transport-lab", "fixture", "--directory"])
            .arg(directory)
            .output()
            .unwrap()
    };
    assert!(run().status.success());
    let before = fs::read(directory.join("nyc.json")).unwrap();
    assert!(!run().status.success());
    assert_eq!(fs::read(directory.join("nyc.json")).unwrap(), before);
    let output = agent()
        .args(["transport-lab", "run", "--config"])
        .arg(directory.join("nyc.json"))
        .arg("--status-file")
        .arg(directory.join("nyc.json"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(directory.join("nyc.json")).unwrap(), before);
}
