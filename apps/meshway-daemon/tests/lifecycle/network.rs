//! Real-process mode boundary; no fake Node or fabricated execution authority.
use super::support::*;
use meshway_ipc::{Command, DataPlane, NetworkMode, Phase, Problem};

#[tokio::test]
async fn unsupported_tun_keeps_ipc_responsive_and_never_opens_unprotected_control() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let directory = root.join("tun-node");
    let mut daemon = Process::start_with_mode(&directory, "http://127.0.0.1:9", "tun");
    let blocked = wait(&daemon.socket, Phase::Blocked).await;
    assert_eq!(blocked.network_mode, NetworkMode::Tun);
    assert_eq!(blocked.data_plane, DataPlane::Failed);
    assert_eq!(blocked.last_error, Some(Problem::UnsupportedMode));
    assert!(blocked.identity.is_none() && blocked.instance.is_none() && blocked.applied.is_none());
    assert!(!directory.join("identity").exists());
    call(
        &daemon.socket,
        Command::Up {
            enrollment_key: None,
            expected_revision: blocked.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    let blocked = wait(&daemon.socket, Phase::Blocked).await;
    assert!(blocked.desired_up);
    assert_eq!(blocked.last_error, Some(Problem::UnsupportedMode));
    assert!(blocked.enrollment_request_id.is_none() && !blocked.authenticated);
    daemon.exit("-TERM").await;
}
