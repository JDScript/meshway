use std::process::Command;

#[test]
fn state_environment_default_and_explicit_override_are_consistent() {
    for arguments in [
        vec![
            "enroll",
            "--controller",
            "http://127.0.0.1:1",
            "--token",
            "invalid",
            "--name",
            "unused",
        ],
        vec!["heartbeat"],
        vec!["sync"],
        vec!["run"],
        vec!["status"],
        vec!["connections", "publish"],
        vec!["connections", "sync"],
        vec!["connections", "status"],
    ] {
        let help = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .env_remove("MESHWAY_STATE")
            .args(&arguments)
            .arg("--help")
            .output()
            .unwrap();
        assert!(help.status.success());
        let help = String::from_utf8(help.stdout).unwrap();
        assert!(help.contains("MESHWAY_STATE"));
        assert!(help.contains(".meshway/agent.json"));
    }
    // Status reads existing state without creating a state directory or lock.
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    let invalid = directory.path().join("invalid");
    std::fs::write(&invalid, b"invalid identity").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&invalid, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let env = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &invalid)
        .arg("status")
        .output()
        .unwrap();
    assert!(!env.status.success());
    assert!(
        String::from_utf8(env.stderr)
            .unwrap()
            .contains("unrecognized or damaged")
    );
    let explicit = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &missing)
        .args(["status", "--state"])
        .arg(&invalid)
        .output()
        .unwrap();
    assert!(!explicit.status.success());
    assert!(
        String::from_utf8(explicit.stderr)
            .unwrap()
            .contains("unrecognized or damaged")
    );
    assert!(!missing.exists());
}

#[cfg(all(feature = "managed-connections", unix))]
#[test]
fn ping_bounds_and_missing_daemon_fail_without_creating_state() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let state = directory.path().join("a");
    for (flag, value) in [
        ("--count", "0"),
        ("--count", "21"),
        ("--timeout-seconds", "0"),
        ("--timeout-seconds", "11"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
            .env("MESHWAY_STATE", &state)
            .args(["ping", "sha", flag, value])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-agent"))
        .env("MESHWAY_STATE", &state)
        .args(["ping", "sha", "--count", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("managed daemon unavailable")
    );
    assert!(!state.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
