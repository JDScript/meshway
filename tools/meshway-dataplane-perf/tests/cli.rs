use std::process::Command;

#[test]
fn manager_stage_requires_fixture_before_tun_or_socket_creation() {
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-dataplane-perf"))
        .args([
            "--stage",
            "manager",
            "--role",
            "a",
            "--bind",
            "127.0.0.1:0",
            "--peer",
            "127.0.0.1:41000",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("requires --fixture")
    );
}

#[test]
fn secure_requires_public_and_private_fixture_before_io() {
    let output = Command::new(env!("CARGO_BIN_EXE_meshway-dataplane-perf"))
        .args([
            "--stage",
            "secure",
            "--role",
            "a",
            "--bind",
            "127.0.0.1:0",
            "--peer",
            "127.0.0.1:41000",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("requires --fixture")
    );
}

#[test]
fn invalid_budget_fails_before_io() {
    for (flag, value) in [
        ("--batch", "0"),
        ("--batch", "1025"),
        ("--mtu", "1279"),
        ("--mtu", "65001"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_meshway-dataplane-perf"))
            .args([
                "--stage",
                "plain",
                "--role",
                "b",
                "--bind",
                "127.0.0.1:0",
                "--peer",
                "127.0.0.1:41000",
                flag,
                value,
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("MTU must be")
        );
    }
}
