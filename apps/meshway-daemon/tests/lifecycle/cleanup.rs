//! Real owner cleanup in a disposable Linux network namespace and private /run.
use super::support::*;
use meshway_dns::SystemDnsOwner;
use meshway_ipc::{Command, DataPlane, Phase, Problem, ResultBody};
use meshway_private_storage::Directory;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires an isolated Linux network namespace, private /run and NET_ADMIN"]
async fn blocked_down_retries_real_dns_reservation_cleanup_without_going_online() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
        Ok("1")
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().canonicalize().unwrap().join("node");
    let mut daemon = Process::start_with_options(
        &path,
        "http://127.0.0.1:9",
        "tun",
        &["--bootstrap-nameserver", "192.0.2.53:53"],
    );
    let initial = wait(&daemon.socket, Phase::Down).await;
    let directory = Arc::new(Directory::open(&path, "fixture.lock").unwrap());
    // A real, same-data-owner DNS reservation is awaiting cleanup. Retain its
    // lock through the first down, then release it as a recoverable conflict.
    // This installs no system resolver configuration or substitute DNS owner.
    let reservation = SystemDnsOwner::reserve(Default::default(), directory.clone())
        .await
        .unwrap();
    call(
        &daemon.socket,
        Command::Up {
            enrollment_key: None,
            expected_revision: initial.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    let online_intent = wait(&daemon.socket, Phase::Retrying).await;
    call(
        &daemon.socket,
        Command::Down {
            expected_revision: online_intent.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    let blocked = wait(&daemon.socket, Phase::Blocked).await;
    assert!(!blocked.desired_up);
    assert_eq!(blocked.last_error, Some(Problem::Dns));
    drop(reservation);
    assert!(
        SystemDnsOwner::has_reservation(directory.clone())
            .await
            .unwrap()
    );

    let id = request_id();
    let retry = Command::Down {
        expected_revision: blocked.revision,
    };
    let ResultBody::Status { status: accepted } = call(&daemon.socket, retry.clone(), id)
        .await
        .unwrap()
        .result
    else {
        panic!("new down retry must be accepted");
    };
    assert_eq!(accepted.revision.get(), blocked.revision.get() + 1);
    assert!(!accepted.desired_up);
    let down = wait(&daemon.socket, Phase::Down).await;
    assert_eq!(down.revision, accepted.revision);
    assert_eq!(down.data_plane, DataPlane::Stopped);
    assert_eq!(down.process_id, initial.process_id);
    assert!(!down.authenticated && down.instance.is_none());
    assert!(!SystemDnsOwner::has_reservation(directory).await.unwrap());
    let ResultBody::Status { status: replay } =
        call(&daemon.socket, retry, id).await.unwrap().result
    else {
        panic!("same request must replay");
    };
    assert_eq!(replay.revision, down.revision);
    assert_eq!(replay.phase, Phase::Down);
    daemon.exit("-TERM").await;
}
