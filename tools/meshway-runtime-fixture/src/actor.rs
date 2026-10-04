use crate::{
    Cli, Error, IO, Result, STARTUP,
    application::{self, Counts, Echo, Subscription, SubscriptionEvidence, Traffic},
    signals::{self, Addresses, Role, Signal, Signals},
};
use meshway_ipc::{DataPlane, NetworkMode, Phase, Status};
use meshway_model::Revision;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
};

pub async fn run(cli: &Cli, role: Role) -> Result<()> {
    let signals = Signals::new(&cli.signals);
    signals.ready("enroll").await?;
    let underlay = signals::underlay(cli).await?;
    underlay_ready().await?;
    let baseline = capture().await?;
    save(cli, "capture-before", &baseline).await?;
    let mut daemon = Command::new(cli.binaries.join("meshwayd"))
        .args([
            "--log-format",
            "json",
            "--log-filter",
            "info,meshway_daemon=debug",
            "--server",
            meshway_packet_fixture::CONTROLLER_ORIGIN,
            "--network-mode",
            "tun",
            "--hostname",
            role.name(),
            "--bootstrap-host",
        ])
        .arg(format!(
            "{}={}",
            meshway_packet_fixture::CONTROLLER_NAME,
            underlay.controller
        ))
        .arg("--data-dir")
        .arg(cli.state.join("node"))
        .env("SSL_CERT_FILE", cli.public.join("server.pem"))
        .env("SSL_CERT_DIR", cli.public.join("empty-cert-dir"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(private_file(
            &cli.evidence.join("daemon.stdout"),
        )?))
        .stderr(Stdio::from(private_file(
            &cli.evidence.join("daemon.stderr"),
        )?))
        .kill_on_drop(true)
        .spawn()?;
    let mut echo = None;
    let result = exercise(cli, role, &signals, &baseline, &mut daemon, &mut echo).await;
    if let Err(error) = &result {
        crate::record_error(cli, "failure.txt", error.as_ref());
        let _ = signals
            .write(&format!("abort-{}", role.name()), &Signal::Ready)
            .await;
        if let Ok(status) = command(cli, "status").await {
            let _ = save(cli, "failure-status", &status).await;
        }
        if let Ok(capture) = capture().await {
            let _ = save(cli, "failure-capture", &capture).await;
        }
        if let Ok(journal) = tokio::fs::read(cli.state.join("node/underlay.json")).await {
            let _ = tokio::fs::write(cli.evidence.join("failure-underlay.json"), journal).await;
        }
    }
    // No early return before every owned actor task and child has been joined.
    let echo_result = match &mut echo {
        Some(echo) => echo.shutdown().await,
        None => Ok(()),
    };
    let down_result = command(cli, "down").await;
    let shutdown_result = terminate(&mut daemon).await;
    result?;
    echo_result?;
    down_result?;
    shutdown_result?;
    Ok(())
}

async fn exercise(
    cli: &Cli,
    role: Role,
    signals: &Signals,
    baseline: &Capture,
    daemon: &mut Child,
    echo: &mut Option<Echo>,
) -> Result<()> {
    tracing::info!(
        phase = "cli-up",
        role = role.name(),
        "starting actual daemon and local CLI"
    );
    tokio::time::timeout(STARTUP, async {
        while !tokio::fs::try_exists(cli.state.join("node/control.sock")).await? {
            if daemon.try_wait()?.is_some() {
                return Err("daemon exited before IPC ready".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<(), Error>(())
    })
    .await??;
    command(cli, "up").await?;
    let status = wait_status(cli, |status| status.enrollment_request_id.is_some()).await?;
    if daemon.id() != Some(status.process_id) {
        return Err("CLI status did not come from spawned daemon".into());
    }
    publish_status(cli, signals, role, "enrollment", status).await?;
    let status = wait_status(cli, |status| status.identity.is_some()).await?;
    let identity = status.identity.clone().ok_or("missing enrolled identity")?;
    publish_status(cli, signals, role, "enrolled", status).await?;
    let Signal::Configuration {
        revision,
        addresses,
    } = signals.wait("go").await?
    else {
        return Err("invalid configuration rendezvous".into());
    };
    if addresses.len() != 3 || Role::ALL.iter().any(|role| !addresses.contains_key(role)) {
        return Err("incomplete managed actor addresses".into());
    }
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "initial", status).await?;
    let tun = owned_tun().await?;
    let tun_index = tun_index(&tun).await?;
    save(cli, "tun-identity", &(&tun, tun_index)).await?;
    let initial_rx = tun_received(&tun).await?;
    route_proof(cli, &addresses[&Role::B], &tun, false, "running").await?;
    save(cli, "capture-running", &capture().await?).await?;
    let underlay = signals::underlay(cli).await?;
    *echo = Some(Echo::start(&addresses[&role], underlay.actors[&role]).await?);
    signals
        .write(&format!("ready-{}", role.name()), &Signal::Ready)
        .await?;
    for actor in Role::ALL {
        signals.ready(&format!("ready-{}", actor.name())).await?;
    }
    let mut traffic = if role == Role::A {
        Some(Traffic::new(&addresses[&Role::A], &addresses[&Role::B]).await?)
    } else {
        None
    };
    if let Some(traffic) = &mut traffic {
        tracing::info!(
            phase = "first-demand",
            "IPv4 UDP creates first actual demand"
        );
        traffic.first_udp().await?;
        save(cli, "first-demand-status", &command(cli, "status").await?).await?;
        traffic.add_demands(&addresses[&Role::A]).await?;
        let sockets = traffic.sockets()?;
        save(cli, "persistent-sockets", &sockets).await?;
        let subscription = Subscription::start(&addresses[&Role::A], &addresses[&Role::B]).await?;
        tracing::info!(
            phase = "persistent-traffic",
            "same sockets and receive-only UDP subscription across 125 seconds"
        );
        let started = Instant::now();
        // Daemon asks for 120 seconds; Server further caps by its instance lease.
        // 125 seconds of active traffic exceeds both initial deadlines, without
        // relying on a rewritten status sample or assuming a renewal succeeded.
        let regular = async {
            while started.elapsed() < Duration::from_secs(125) {
                traffic.exchange().await?;
                verify_tun_index(&tun, tun_index).await?;
                if traffic.sockets()? != sockets {
                    return Err("persistent socket identity changed".into());
                }
                echo.as_mut().ok_or("missing echo owner")?.check()?;
                if daemon.try_wait()?.is_some() {
                    return Err("daemon exited during traffic".into());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Ok::<(), Error>(())
        };
        // Both are ordinary futures, so failure cancels the other socket task
        // without detaching a producer. receive_only consumes/drops its socket
        // after a fresh post-125s reply; restoration never subscribes again.
        let (_, reception) = tokio::try_join!(regular, subscription.receive_only())?;
        let inbound = inbound_stream_evidence(cli, reception).await?;
        save(cli, "inbound-stream", &inbound).await?;
        save(cli, "persistent-traffic-counts", &traffic.counts).await?;
        save(
            cli,
            "persistent-packet-evidence",
            &traffic.evidence(started.elapsed())?,
        )
        .await?;
        save(cli, "after-renewal-status", &command(cli, "status").await?).await?;
        signals.write("traffic", &Signal::Ready).await?;
    } else {
        // Include the additional subscription's bounded initial warmup; the
        // process-wide deadline still reserves the existing cleanup budget.
        let deadline = Instant::now() + crate::remaining(Duration::from_secs(250))?;
        loop {
            echo.as_mut().ok_or("missing echo owner")?.check()?;
            verify_tun_index(&tun, tun_index).await?;
            if role == Role::R && tun_received(&tun).await? != initial_rx {
                return Err(
                    "relay TUN received business plaintext during persistent traffic".into(),
                );
            }
            if let Some(signal) = signals.read("traffic").await? {
                if !matches!(signal, Signal::Ready) {
                    return Err("invalid traffic rendezvous".into());
                }
                break;
            }
            if Instant::now() >= deadline {
                return Err("persistent traffic rendezvous deadline exceeded".into());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    save(
        cli,
        "tun-received-after-traffic",
        &tun_received(&tun).await?,
    )
    .await?;
    signals
        .write(&format!("traffic-ready-{}", role.name()), &Signal::Ready)
        .await?;
    let Signal::Revision { revision } = signals
        .wait_for("revoked", Duration::from_secs(175))
        .await?
    else {
        return Err("invalid revocation rendezvous".into());
    };
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "revoked", status).await?;
    if let Some(traffic) = &mut traffic {
        route_proof(cli, &addresses[&Role::B], &tun, false, "revoked").await?;
        traffic.blocked().await?;
        signals.write("revoked-traffic", &Signal::Ready).await?;
    }
    let Signal::Revision { revision } = signals.wait("restored").await? else {
        return Err("invalid restoration rendezvous".into());
    };
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "restored", status).await?;
    if role == Role::R {
        let v4 = ip(&["-4", "route", "get", &addresses[&Role::B].ipv4.to_string()]).await?;
        let v6 = ip(&["-6", "route", "get", &addresses[&Role::B].ipv6.to_string()]).await?;
        let v4_text = String::from_utf8(v4.stdout)?;
        let v6_error = String::from_utf8(v6.stderr)?;
        if !v4.status.success()
            || !v4_text
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .any(|pair| pair == ["dev", tun.as_str()])
            || v6.status.success()
            || !(v6_error.contains("unreachable") || v6_error.contains("No route"))
        {
            return Err("unassigned relay IPv6 family did not remain negative-only".into());
        }
        save(cli, "nullable-family-capture", &(v4_text, v6_error)).await?;
    }
    if let Some(traffic) = &mut traffic {
        traffic.restored().await?;
    }
    let received = tun_received(&tun)
        .await?
        .checked_sub(initial_rx)
        .ok_or("TUN counter went backwards")?;
    save(
        cli,
        "tun-received-packets",
        &(initial_rx, initial_rx + received),
    )
    .await?;
    let pre_outage = command(cli, "status").await?;
    save(cli, "before-outage-status", &pre_outage).await?;
    signals
        .write(&format!("lease-ready-{}", role.name()), &Signal::Ready)
        .await?;
    signals.ready("offline").await?;
    let fresh = command(cli, "status").await?;
    let lease = fresh
        .instance
        .as_ref()
        .or(pre_outage.instance.as_ref())
        .ok_or("no actual instance lease at Server stop")?;
    let expiry = lease
        .execution_expires_at_ms
        .ok_or("no execution lease at Server stop")?
        .min(lease.expires_at_ms);
    save(cli, "lease-at-outage", &fresh).await?;
    let now = now_ms()?;
    if expiry <= now || expiry.saturating_sub(now) > 70_000 {
        return Err("unexpected actual lease deadline".into());
    }
    let wait = Duration::from_millis(expiry - now + 500);
    if crate::remaining(wait)? < wait {
        return Err("lease expiry exceeds fixture deadline".into());
    }
    tokio::time::sleep(wait).await;
    let suspended = wait_status(cli, |status| {
        status.data_plane == DataPlane::Suspended && status.desired_up
    })
    .await?;
    save(cli, "lease-expired-status", &suspended).await?;
    // Prove the unmarked destination still cannot escape via Docker's default
    // route. A held TUN or an unreachable policy route is acceptable here.
    route_proof(cli, &addresses[&Role::B], &tun, true, "suspended").await?;
    if let Some(traffic) = &mut traffic {
        traffic.blocked().await?;
    }
    application::underlay_probe(underlay.actors[&Role::B]).await?;
    echo.as_mut().ok_or("missing echo owner")?.check()?;
    let counts = traffic.as_ref().map_or_else(
        || echo.as_ref().map_or(Counts::default(), Echo::counts),
        |traffic| traffic.counts,
    );
    if role == Role::R
        && (received != 0
            || counts.udp4 != 0
            || counts.udp6 != 0
            || counts.tcp4 != 0
            || counts.tcp6 != 0)
    {
        return Err("relay delivered business plaintext to its TUN/application".into());
    }
    // Coordinate after all outage probes so B cannot tear down its underlay
    // application while A/R still need their independent reachability check.
    signals
        .write(&format!("outage-{}", role.name()), &Signal::Ready)
        .await?;
    for actor in Role::ALL {
        signals.ready(&format!("outage-{}", actor.name())).await?;
    }
    let private_identity = cli.state.join("node/identity/control.json");
    let before = tokio::fs::read(&private_identity).await?;
    command(cli, "down").await?;
    let status = wait_status(cli, |status| {
        status.phase == Phase::Down && !status.desired_up && status.data_plane == DataPlane::Stopped
    })
    .await?;
    save(cli, "down-status", &status).await?;
    if tokio::fs::try_exists(Path::new("/sys/class/net").join(&tun)).await? {
        return Err("CLI down left TUN installed".into());
    }
    let after = capture().await?;
    save(cli, "capture-after", &after).await?;
    if &after != baseline {
        return Err("CLI down left owned capture routes/rules".into());
    }
    if before != tokio::fs::read(private_identity).await? {
        return Err("CLI down altered private identity record".into());
    }
    if status
        .identity
        .as_ref()
        .is_some_and(|current| current != &identity)
    {
        return Err("identity changed on down".into());
    }
    signals
        .write(
            &format!("report-{}", role.name()),
            &Signal::Report {
                role,
                node: identity.node_id,
                udp4: counts.udp4,
                udp6: counts.udp6,
                tcp4: counts.tcp4,
                tcp6: counts.tcp6,
                relay_tun_received: (role == Role::R).then_some(received),
                tun_removed: true,
                capture_removed: true,
                identity_preserved: true,
            },
        )
        .await?;
    tracing::info!(
        phase = "verified",
        role = role.name(),
        "runtime packet/control/invalidation/cleanup evidence complete"
    );
    Ok(())
}

async fn publish_status(
    cli: &Cli,
    signals: &Signals,
    role: Role,
    stage: &str,
    status: Status,
) -> Result<()> {
    save(cli, &format!("status-{stage}"), &status).await?;
    signals
        .write(
            &format!("{stage}-{}", role.name()),
            &Signal::Status {
                role,
                status: Box::new(status),
            },
        )
        .await
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "lowercase", deny_unknown_fields)]
enum DemandAcknowledgement {
    Create {
        #[serde(rename = "message")]
        _message: String,
        request_id: meshway_model::RequestId,
        protocol: String,
        #[serde(default)]
        port: Option<u16>,
        expires_at_ms: u64,
    },
    Renew {
        #[serde(rename = "message")]
        _message: String,
        request_id: meshway_model::RequestId,
        expires_at_ms: u64,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InboundStreamEvidence {
    #[serde(flatten)]
    reception: SubscriptionEvidence,
    original_demand_id: meshway_model::RequestId,
    initial_expiry_ms: u64,
    later_expiry_ms: u64,
    acknowledged_creates: u64,
    acknowledged_renewals: u64,
}

async fn inbound_stream_evidence(
    cli: &Cli,
    reception: SubscriptionEvidence,
) -> Result<InboundStreamEvidence> {
    // This parses this actor's known local JSON tracing output, never a Server
    // response/authorization DTO. Selected event fields use a closed schema.
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut file = tokio::fs::File::open(cli.evidence.join("daemon.stderr"))
        .await?
        .take(LIMIT + 1);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > LIMIT {
        return Err("daemon acknowledgement log exceeds evidence bound".into());
    }
    let text = std::str::from_utf8(&bytes)?;
    let end = text
        .rfind('\n')
        .ok_or("daemon acknowledgement log has no complete event")?;
    let mut original = None;
    let mut initial_expiry = None;
    let mut creates = 0_u64;
    let mut renewals = Vec::new();
    for line in text[..end].lines() {
        let event: serde_json::Value = serde_json::from_str(line)?;
        if event.get("target").and_then(serde_json::Value::as_str)
            != Some("meshway_daemon::runtime::network")
            || event
                .get("fields")
                .and_then(|fields| fields.get("message"))
                .and_then(serde_json::Value::as_str)
                != Some("Demand request acknowledged")
        {
            continue;
        }
        let acknowledgement: DemandAcknowledgement = serde_json::from_value(
            event
                .get("fields")
                .ok_or("missing acknowledgement fields")?
                .clone(),
        )?;
        match acknowledgement {
            DemandAcknowledgement::Create {
                request_id,
                protocol,
                port: Some(application::SUBSCRIPTION_PORT),
                expires_at_ms,
                ..
            } => {
                if protocol != "Udp" {
                    return Err("subscription demand used unexpected protocol".into());
                }
                if original.is_some_and(|original| original != request_id) {
                    return Err("receive-only stream created a replacement demand".into());
                }
                original = Some(request_id);
                initial_expiry.get_or_insert(expires_at_ms);
                creates += 1;
            }
            DemandAcknowledgement::Create { .. } => {}
            DemandAcknowledgement::Renew {
                request_id,
                expires_at_ms,
                ..
            } => renewals.push((request_id, expires_at_ms)),
        }
    }
    let original = original.ok_or("subscription has no acknowledged original demand")?;
    let initial_expiry =
        initial_expiry.ok_or("subscription original permit expiry is unmeasured")?;
    let extended: Vec<_> = renewals
        .into_iter()
        .filter(|(id, expiry)| *id == original && *expiry > initial_expiry)
        .collect();
    let later_expiry = extended
        .iter()
        .map(|(_, expiry)| *expiry)
        .max()
        .ok_or("original receive-only demand has no acknowledged extension")?;
    if reception.sent_initial_count == 0
        || reception.post_start_sent_count != reception.sent_initial_count
        || reception.receive_only_elapsed_ms < 125_000
        || reception.end_wall_ms <= initial_expiry
        || reception.max_sequence == 0
        || reception.received_count == 0
    {
        return Err("receive-only stream lacks post-original-expiry evidence".into());
    }
    Ok(InboundStreamEvidence {
        reception,
        original_demand_id: original,
        initial_expiry_ms: initial_expiry,
        later_expiry_ms: later_expiry,
        acknowledged_creates: creates,
        acknowledged_renewals: extended.len().try_into()?,
    })
}

async fn running(cli: &Cli, revision: Revision) -> Result<Status> {
    wait_status(cli, |status| {
        status.phase == Phase::Running
            && status.data_plane == DataPlane::Ready
            && status.network_mode == NetworkMode::Tun
            && status.authenticated
            && status.desired_up
            && status
                .saved
                .as_ref()
                .is_some_and(|saved| saved.revision == revision)
            && status
                .applied
                .as_ref()
                .is_some_and(|applied| applied.revision == revision)
    })
    .await
}

async fn wait_status(cli: &Cli, predicate: impl Fn(&Status) -> bool) -> Result<Status> {
    let deadline = Instant::now() + crate::remaining(STARTUP)?;
    loop {
        let status = command(cli, "status").await?;
        if predicate(&status) {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err("daemon status deadline exceeded".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn command(cli: &Cli, operation: &str) -> Result<Status> {
    let deadline = if operation == "down" {
        IO
    } else {
        crate::remaining(IO)?
    };
    let mut child = Command::new(cli.binaries.join("meshway"))
        .arg("--socket")
        .arg(cli.state.join("node/control.sock"))
        .args(["--json", operation])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(private_file(&cli.evidence.join("cli.stderr"))?))
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("CLI stdout missing")?
        .take(64 * 1024 + 1);
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(deadline, async {
        let (_, status) = tokio::try_join!(stdout.read_to_end(&mut bytes), child.wait())?;
        if !status.success() || bytes.len() > 64 * 1024 {
            return Err("CLI command failed or exceeded output bound".into());
        }
        Ok::<Status, Error>(serde_json::from_slice(&bytes)?)
    })
    .await;
    if !matches!(&result, Ok(Ok(_))) {
        kill_join(&mut child).await?;
    }
    result?
}

fn private_file(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?)
}

async fn terminate(child: &mut Child) -> Result<()> {
    if child.try_wait()?.is_some() {
        child.wait().await?;
        return Err("daemon exited before fixture shutdown".into());
    }
    let pid = child.id().ok_or("daemon PID absent")?;
    let sent: Result<()> = async {
        let mut signal = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .kill_on_drop(true)
            .spawn()?;
        let sent = tokio::time::timeout(IO, signal.wait()).await;
        if !matches!(&sent, Ok(Ok(_))) {
            kill_join(&mut signal).await?;
        }
        if !sent??.success() {
            return Err("daemon termination signal failed".into());
        }
        Ok(())
    }
    .await;
    let joined = tokio::time::timeout(IO, child.wait()).await;
    if !matches!(&joined, Ok(Ok(_))) {
        kill_join(child).await?;
    }
    sent?;
    if !joined??.success() {
        return Err("daemon shutdown failed".into());
    }
    Ok(())
}

async fn kill_join(child: &mut Child) -> Result<()> {
    let killed = child.start_kill();
    let joined = child.wait().await;
    killed?;
    joined?;
    Ok(())
}

#[derive(Serialize, PartialEq, Eq)]
struct Capture(BTreeMap<String, Vec<String>>);

/// Docker's ordinary IPv6 DAD may finish after container start and add its
/// link-local route. Capture a settled underlay before starting the daemon so
/// exact down comparison measures owned changes, not that kernel transition.
async fn underlay_ready() -> Result<()> {
    tokio::time::timeout(crate::remaining(IO)?, async {
        loop {
            let contents = tokio::fs::read_to_string("/proc/net/if_inet6").await?;
            let mut global = false;
            let mut local = false;
            let mut ready = true;
            for line in contents.lines() {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() != 6 {
                    return Err("invalid IPv6 interface inspection".into());
                }
                if fields[5] != "eth0" {
                    continue;
                }
                let flags = u32::from_str_radix(fields[4], 16)?;
                let scope = u32::from_str_radix(fields[3], 16)?;
                if flags & 0x08 != 0 {
                    return Err("isolated underlay IPv6 DAD failed".into());
                }
                ready &= flags & 0x40 == 0;
                global |= scope == 0;
                local |= scope == 0x20;
            }
            if ready && global && local {
                return Ok::<(), Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}

async fn capture() -> Result<Capture> {
    let mut output = BTreeMap::new();
    for family in ["-4", "-6"] {
        for (kind, args) in [
            ("rule", vec![family, "rule", "show"]),
            ("route", vec![family, "route", "show", "table", "all"]),
        ] {
            let result = ip(&args).await?;
            if !result.status.success() {
                return Err("iproute capture inspection unavailable".into());
            }
            let mut lines: Vec<_> = String::from_utf8(result.stdout)?
                .lines()
                .map(str::to_owned)
                .collect();
            lines.sort();
            output.insert(format!("{family}-{kind}"), lines);
        }
    }
    Ok(Capture(output))
}

async fn route_proof(
    cli: &Cli,
    destination: &Addresses,
    tun: &str,
    suspended: bool,
    stage: &str,
) -> Result<()> {
    for (family, address) in [
        ("-4", destination.ipv4.to_string()),
        ("-6", destination.ipv6.to_string()),
    ] {
        let result = ip(&[family, "route", "get", &address]).await?;
        let text = String::from_utf8(result.stdout)?;
        let error = String::from_utf8(result.stderr)?;
        save(
            cli,
            &format!("route-{stage}-{family}"),
            &(result.status.code(), &text, &error),
        )
        .await?;
        if !result.status.success() {
            if suspended && (error.contains("unreachable") || error.contains("No route")) {
                continue;
            }
            return Err("managed route lookup failed".into());
        }
        let words: Vec<_> = text.split_whitespace().collect();
        if !words.windows(2).any(|pair| pair == ["dev", tun]) {
            // Destination B is local on B: local delivery to lo is correct.
            if words.first() == Some(&"local") && words.windows(2).any(|pair| pair == ["dev", "lo"])
            {
                continue;
            }
            return Err("unmarked managed destination escaped owned capture".into());
        }
    }
    Ok(())
}

async fn ip(args: &[&str]) -> Result<std::process::Output> {
    let deadline = crate::remaining(IO)?;
    // ip only inspects this actor's namespace. It never changes kernel state.
    let mut child = Command::new("ip")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("ip stdout absent")?
        .take(1024 * 1024);
    let mut stderr = child
        .stderr
        .take()
        .ok_or("ip stderr absent")?
        .take(64 * 1024);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let result = tokio::time::timeout(deadline, async {
        let (_, _, status) = tokio::try_join!(
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            child.wait()
        )?;
        Ok::<_, Error>(std::process::Output {
            status,
            stdout: out,
            stderr: err,
        })
    })
    .await;
    if !matches!(&result, Ok(Ok(_))) {
        kill_join(&mut child).await?;
    }
    result?
}

async fn owned_tun() -> Result<String> {
    let mut entries = tokio::fs::read_dir("/sys/class/net").await?;
    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if tokio::fs::try_exists(entry.path().join("tun_flags")).await? {
            names.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "invalid TUN name")?,
            );
        }
    }
    if names.len() != 1 {
        return Err("expected exactly one daemon TUN".into());
    }
    names.pop().ok_or_else(|| "missing daemon TUN".into())
}

async fn tun_received(name: &str) -> Result<u64> {
    Ok(tokio::fs::read_to_string(
        PathBuf::from("/sys/class/net")
            .join(name)
            .join("statistics/rx_packets"),
    )
    .await?
    .trim()
    .parse()?)
}

async fn tun_index(name: &str) -> Result<u32> {
    Ok(
        tokio::fs::read_to_string(PathBuf::from("/sys/class/net").join(name).join("ifindex"))
            .await?
            .trim()
            .parse()?,
    )
}

async fn verify_tun_index(name: &str, expected: u32) -> Result<()> {
    if tun_index(name).await? != expected {
        return Err("TUN identity changed during a live unchanged-capture handoff".into());
    }
    Ok(())
}

async fn save(cli: &Cli, name: &str, value: &impl Serialize) -> Result<()> {
    tokio::fs::write(
        cli.evidence.join(format!("{name}.json")),
        serde_json::to_vec_pretty(value)?,
    )
    .await?;
    Ok(())
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
