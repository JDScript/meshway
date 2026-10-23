//! DNS-2b: the real daemon on a systemd-resolved that never delivers the
//! probe to the anchor's loopback bind (Ubuntu 22.04's 249.11, the
//! `jammy/resolv-conf` row's DNS image under `dev/dns-fixture/namespace.sh`).
//! It stays `retrying · dns` with the probe's requirement and retries the
//! probe with a growing pause: at most 6 probes in the first 60 s (1, 2, 4,
//! 8, 16 s pauses plus the 5 s probe each; the fixed 1 s pause gave 11).
use super::*;

/// A probe failure's bind port names one probe (each probe binds anew).
fn probe_port(detail: &str) -> Option<&str> {
    let rest = detail.split("no query reached 127.0.0.1:").nth(1)?;
    rest.split(' ').next()
}

#[tokio::test]
#[ignore = "requires the jammy DNS image's own D-Bus and systemd-resolved, NET_ADMIN, /dev/net/tun and private /run"]
async fn an_unusable_resolved_is_probed_with_a_growing_pause() {
    assert_eq!(std::env::var("MESHWAY_DNS_ISOLATED").as_deref(), Ok("1"));
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let server = Server::start(root.join("server")).await;
    let (user, network) = server.owner().await;
    let path = root.join("node");
    let mut daemon = Process::start_with_options(
        &path,
        &server.origin,
        "tun",
        &["--bootstrap-nameserver", "192.0.2.53:53"],
    );
    let initial = wait(&daemon.socket, Phase::Down).await;
    let up = Command::Up {
        enrollment_key: None,
        expected_revision: initial.revision,
        reenroll: false,
    };
    call(&daemon.socket, up, request_id()).await.unwrap();
    let pending = wait(&daemon.socket, Phase::AwaitingApproval).await;
    server
        .client
        .approve_device_authorization(
            &ApproveDeviceAuthorizationPath {
                enrollment_request_id: pending.enrollment_request_id.unwrap(),
            },
            &ApproveEnrollmentRequest {
                network_id: network,
            },
            user.expose(),
        )
        .await
        .unwrap();
    // Each probe failure, when it first shows, with the next re-proof then.
    let mut probes: Vec<(Instant, String, Option<chrono::DateTime<chrono::Utc>>)> = Vec::new();
    let mut detail = None;
    let started = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let Ok(status) = status(&daemon.socket).await else {
            continue;
        };
        assert_ne!(status.phase, Phase::Blocked, "{status:?}");
        if let Some(text) = status.last_error_detail.as_deref()
            && let Some(port) = probe_port(text)
            && probes.last().is_none_or(|(_, last, _)| last != port)
        {
            assert_eq!(status.phase, Phase::Retrying, "{status:?}");
            assert_eq!(status.last_error, Some(meshway_protocol::ipc::Problem::Dns));
            println!("{:>6.1} s {text}", started.elapsed().as_secs_f64());
            probes.push((Instant::now(), port.to_owned(), status.retry_at));
            detail = Some(text.to_owned());
        }
        if probes
            .first()
            .is_some_and(|(first, _, _)| first.elapsed() > Duration::from_secs(62))
        {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "no probe failure: {probes:?}"
        );
    }
    let detail = detail.unwrap();
    assert!(
        detail.contains("systemd-resolved 253 or later")
            && detail.contains("see install.md: Ubuntu 22.04"),
        "{detail}"
    );
    let first = probes[0].0;
    let within: Vec<_> = probes
        .iter()
        .filter(|(at, _, _)| at.duration_since(first) <= Duration::from_secs(60))
        .collect();
    let gaps: Vec<f64> = probes
        .windows(2)
        .map(|pair| pair[1].0.duration_since(pair[0].0).as_secs_f64())
        .collect();
    println!(
        "probes in the first 60 s: {}; gaps {gaps:.1?}",
        within.len()
    );
    assert!(
        (4..=6).contains(&within.len()),
        "{} probes in 60 s, gaps {gaps:?}",
        within.len()
    );
    // The pause doubles: each gap is longer than the one before.
    for pair in gaps.windows(2).take(4) {
        assert!(pair[1] >= pair[0] + 0.5, "gaps {gaps:?}");
    }
    // Status's retryAt is the grown re-proof time: after the fourth failure
    // at least 8 s ahead.
    let (at, _, retry_at) = &probes[3];
    let ahead = retry_at.expect("retryAt while retrying") - chrono::Utc::now()
        + chrono::Duration::from_std(at.elapsed()).unwrap();
    assert!(ahead >= chrono::Duration::milliseconds(7_500), "{ahead}");
    let current = status(&daemon.socket).await.unwrap();
    call(
        &daemon.socket,
        Command::Down {
            expected_revision: current.revision,
        },
        request_id(),
    )
    .await
    .unwrap();
    wait_matching(&daemon.socket, |status| {
        status.phase == Phase::Down
            && status.data_plane == meshway_protocol::ipc::DataPlane::Stopped
    })
    .await;
    daemon.exit("-TERM").await;
    server.close().await;
}
