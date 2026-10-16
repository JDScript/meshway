mod interface_change;
mod reconnect;
pub mod relay;
mod scenario;

use crate::{
    Cli, Error, IO, Result, STARTUP,
    application::{self, Counts, Echo, Subscription, SubscriptionEvidence, Traffic},
    signals::{self, Addresses, Role, Signal, Signals},
};
use meshway_model::Revision;
use meshway_protocol::ipc::{DataPlane, NetworkMode, Phase, Status};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
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
    if cli.traversal == crate::TraversalMode::Nat {
        if std::env::var("MESHWAY_TRAVERSAL_OWNED").as_deref() != Ok("1") {
            return Err("NAT route setup requires the owned fixture namespace".into());
        }
        save(cli, "capture-before-fixture-route", &capture().await?).await?;
        if let Some(router) = underlay.routers.get(&role) {
            let result = ip(&[
                "-4",
                "route",
                "replace",
                "default",
                "via",
                &router.private.to_string(),
                "dev",
                "eth0",
            ])
            .await?;
            if !result.status.success() {
                return Err("fixture-owned NAT default route setup failed".into());
            }
        } else if role != Role::B {
            return Err("NAT actor lacks its owned router".into());
        }
    }
    let baseline = capture().await?;
    save(cli, "capture-before", &baseline).await?;
    let mut dns = crate::dns::Probe::start(cli, role).await?;
    let mut held = (cli.held_listener_port() && role == Role::B)
        .then(|| std::net::UdpSocket::bind(HELD_LISTENER))
        .transpose()?;
    let mut daemon = spawn_daemon(cli, role, underlay.controller)?;
    let mut echo = None;
    let result = exercise(
        cli,
        role,
        &signals,
        &baseline,
        &mut daemon,
        &mut echo,
        &mut dns,
        &mut held,
    )
    .await;
    drop(held);
    if let Err(error) = &result {
        crate::record_error(cli, "failure.txt", error.as_ref());
        if let Some(dns) = &dns {
            let _ = dns.save(cli).await;
        }
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
        let _ = save_dns_journals(cli).await;
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

fn spawn_daemon(cli: &Cli, role: Role, controller: std::net::Ipv4Addr) -> Result<Child> {
    spawn_named_daemon(cli, role.name(), controller)
}

pub(super) fn spawn_named_daemon(
    cli: &Cli,
    name: &str,
    controller: impl Into<std::net::IpAddr>,
) -> Result<Child> {
    let controller: std::net::IpAddr = controller.into();
    let mut daemon_command = Command::new(cli.binaries.join("meshwayd"));
    let filter = "info,meshway_underlay=debug,meshway_daemon=debug,meshway_dataplane::secure=debug,meshway_dataplane::flow=debug,meshway_connectivity::manager=debug,meshway_control::netmap=debug,meshway_dns=debug,meshway_daemon::runtime::network::circuit_packet=debug";
    if cli.traversal == crate::TraversalMode::Exit {
        daemon_command.args(["--dns-backend", "resolved", "--dns-bind", "127.0.0.1:5354"]);
    } else if cli.dns_backend.enabled() {
        daemon_command.args([
            "--dns-backend",
            cli.dns_backend.name(),
            "--dns-bind",
            cli.dns_backend.bind(),
        ]);
    }
    Ok(daemon_command
        .args([
            "--log-format",
            "json",
            "--log-filter",
            filter,
            "--server",
            meshway_packet_fixture::CONTROLLER_ORIGIN,
            "--network-mode",
            "tun",
            "--hostname",
            name,
            "--bootstrap-host",
        ])
        .arg(format!(
            "{}={}",
            meshway_packet_fixture::CONTROLLER_NAME,
            controller
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
        .spawn()?)
}

/// The listener every Native UDP actor declares.
const HELD_LISTENER: &str = "0.0.0.0:35000";

/// While another socket holds the listener port the Node reports why, and
/// after it is released the Node binds it on its own: the content applies
/// cleanly and the port is the daemon's.
async fn release_held_listener(cli: &Cli, held: std::net::UdpSocket) -> Result<()> {
    let unavailable = wait_status(cli, |status| {
        status.last_error == Some(meshway_protocol::ipc::Problem::Transport)
            && status
                .last_error_detail
                .as_deref()
                .is_some_and(|detail| detail.contains(&format!("cannot bind {HELD_LISTENER}")))
    })
    .await?;
    save(cli, "held-listener-unavailable-status", &unavailable).await?;
    tokio::time::sleep(Duration::from_secs(3)).await;
    drop(held);
    let released = Instant::now();
    let recovered = wait_status_for(cli, Duration::from_secs(90), |status| {
        status.last_error.is_none() && status.data_plane == DataPlane::Ready
    })
    .await?;
    let owned = matches!(
        std::net::UdpSocket::bind(HELD_LISTENER),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse
    );
    save(
        cli,
        "held-listener-recovered",
        &serde_json::json!({
            "status": recovered,
            "recoveredMs": released.elapsed().as_millis() as u64,
            "daemonHoldsPort": owned,
        }),
    )
    .await?;
    if !owned {
        return Err("the released listener port was not bound by the daemon".into());
    }
    tracing::info!(
        phase = "held-listener-recovered",
        recovered_ms = released.elapsed().as_millis() as u64,
        "listener bound after its port was released"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn exercise(
    cli: &Cli,
    role: Role,
    signals: &Signals,
    baseline: &Capture,
    daemon: &mut Child,
    echo: &mut Option<Echo>,
    dns: &mut Option<crate::dns::Probe>,
    held: &mut Option<std::net::UdpSocket>,
) -> Result<()> {
    tracing::info!(
        phase = "cli-up",
        role = role.name(),
        "starting actual daemon and local CLI"
    );
    wait_ipc(cli, daemon).await?;
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
        dns_domain,
    } = signals.wait("go").await?
    else {
        return Err("invalid configuration rendezvous".into());
    };
    if addresses.len() != 3 || Role::ALL.iter().any(|role| !addresses.contains_key(role)) {
        return Err("incomplete managed actor addresses".into());
    }
    if let Some(held) = held.take() {
        release_held_listener(cli, held).await?;
    }
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "initial", status).await?;
    if let Some(dns) = dns {
        dns.active(
            cli,
            role,
            &dns_domain,
            &addresses[&Role::B],
            "before-restart",
            true,
        )
        .await?;
        terminate(daemon).await?;
        dns.unavailable(cli, role, &dns_domain, "restart-gap")
            .await?;
        *daemon = spawn_daemon(cli, role, signals::underlay(cli).await?.controller)?;
        wait_ipc(cli, daemon).await?;
        let status = running(cli, revision).await?;
        if status.identity.as_ref() != Some(&identity)
            || daemon.id() != Some(status.process_id)
            || !status.desired_up
        {
            return Err(
                "daemon restart did not preserve the same identity and desired-up intent".into(),
            );
        }
        save(cli, "dns-restart-status", &status).await?;
        dns.active(
            cli,
            role,
            &dns_domain,
            &addresses[&Role::B],
            "after-restart",
            true,
        )
        .await?;
    }
    let mut tun = owned_tun().await?;
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
    if let Some(dns) = dns {
        dns.active(
            cli,
            role,
            &dns_domain,
            &addresses[&Role::B],
            "initial",
            true,
        )
        .await?;
    }
    let mut traffic = if role == Role::A {
        Some(Traffic::new(&addresses[&Role::A], &addresses[&Role::B]).await?)
    } else {
        None
    };
    // Every matrix mode exercises diagnostics before the first OS business send.
    let mut ping = if role == Role::A {
        Some(crate::ping_cli::initial(cli, signals, &addresses).await?)
    } else {
        if !matches!(
            signals
                .wait_for("ping-shared-verified", crate::ping::INITIAL_LIMIT)
                .await?,
            Signal::Ready
        ) {
            return Err("invalid initial diagnostic completion".into());
        }
        None
    };
    let mut fallback_evidence = None;
    if let Some(traffic) = &mut traffic {
        tracing::info!(
            phase = "first-flow",
            "IPv4 UDP is admitted locally on the installed netmap"
        );
        traffic.first_udp().await?;
        save(cli, "first-flow-status", &command(cli, "status").await?).await?;
        if cli.traversal == crate::TraversalMode::Nat {
            crate::traversal::publish_installations(cli, signals, role).await?;
            signals.write("traversal-first", &Signal::Ready).await?;
            // Record actual acquisition before starting the remaining traffic.
            signals.ready("traversal-verified").await?;
        }
        traffic.open_remaining_flows(&addresses[&Role::A]).await?;
        traffic.mtu_conformance().await?;
        let sockets = traffic.sockets()?;
        save(cli, "persistent-sockets", &sockets).await?;
        let subscription_netmap = command(cli, "status")
            .await?
            .netmap
            .ok_or("subscription has no installed netmap lease")?;
        let subscription = Subscription::start(&addresses[&Role::A], &addresses[&Role::B]).await?;
        let fallback_before = if cli.traversal_fallback {
            // Prove both TCP streams actually echo before starting the clock.
            traffic.exchange().await?;
            let sample = crate::fallback::sample(cli, traffic, Duration::ZERO, "before").await?;
            signals.write("persistent-start", &Signal::Ready).await?;
            Some(sample)
        } else {
            None
        };
        let subscription_observation = subscription.observation();
        tracing::info!(
            phase = "persistent-traffic",
            seconds = cli.traffic_duration().as_secs(),
            "same sockets and receive-only UDP subscription"
        );
        let started = Instant::now();
        // The same sockets must survive the traffic window (one renewal and
        // one lease validity), independently of HTTP instance renewal. The signed netmap is their execution authority.
        let regular = async {
            while started.elapsed() < cli.traffic_duration() {
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
        // without detaching a producer. Snapshot failed reception before dropping
        // the original socket; restoration never subscribes again.
        let result = tokio::try_join!(regular, subscription.receive_only(cli.traffic_duration()));
        let failure = if result.is_err() {
            subscription_observation
                .lock()
                .ok()
                .and_then(|mut value| value.take())
        } else {
            None
        };
        // Preserve the original test failure even if evidence storage fails.
        if let Some(failure) = failure
            && let Ok(file) = private_file(&cli.evidence.join("failure-inbound-stream.json"))
        {
            let _ = serde_json::to_writer_pretty(file, &failure);
        }
        let (_, reception) = result?;
        if let Some(before) = fallback_before {
            signals.ready("fallback-verified").await?;
            let after = crate::fallback::sample(cli, traffic, started.elapsed(), "after").await?;
            fallback_evidence = Some(Box::new(
                crate::fallback::verify(cli, signals, before, after).await?,
            ));
        }
        let inbound = inbound_stream_evidence(cli, reception, subscription_netmap).await?;
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
        let deadline = Instant::now() + crate::remaining(cli.traffic_wait())?;
        let mut installations_published = false;
        loop {
            echo.as_mut().ok_or("missing echo owner")?.check()?;
            verify_tun_index(&tun, tun_index).await?;
            if cli.traversal == crate::TraversalMode::Nat
                && !installations_published
                && matches!(signals.read("traversal-first").await?, Some(Signal::Ready))
            {
                crate::traversal::publish_installations(cli, signals, role).await?;
                installations_published = true;
            }
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
    if let Some(netmap_scenario) = cli.netmap_scenario {
        let tun = scenario::run(
            netmap_scenario,
            scenario::Context {
                cli,
                role,
                signals,
                daemon,
                addresses: &addresses,
                revision,
                traffic: &mut traffic,
                tun,
                tun_index,
            },
        )
        .await?;
        return scenario::down(cli, role, signals, &tun, baseline).await;
    }
    let Signal::Revision { revision } = signals.wait("ping-icmp-revoked").await? else {
        return Err("invalid ICMP-only revocation boundary".into());
    };
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "icmp-revoked", status).await?;
    if let Some(traffic) = &mut traffic {
        let path = crate::fallback::original_path(signals).await?;
        let run = crate::ping_cli::run(cli, crate::ping::Stage::UdpTcpOnly, "b", &path).await?;
        // UDP/TCP remain authorized and actually work while ICMP is denied.
        traffic.exchange().await?;
        signals
            .write(
                "ping-udp-tcp-only",
                &Signal::PingRun {
                    run: Box::new(run.clone()),
                },
            )
            .await?;
        ping.as_mut()
            .ok_or("missing source ping evidence")?
            .runs
            .push(run);
    }
    let Signal::Revision { revision } = signals
        .wait_for("revoked", Duration::from_secs(175))
        .await?
    else {
        return Err("invalid revocation rendezvous".into());
    };
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "revoked", status).await?;
    if let Some(dns) = dns {
        dns.active(
            cli,
            role,
            &dns_domain,
            &addresses[&Role::B],
            "revoked",
            false,
        )
        .await?;
    }
    if let Some(traffic) = &mut traffic {
        route_proof(cli, &addresses[&Role::B], &tun, false, "revoked").await?;
        traffic.blocked().await?;
        let path = crate::fallback::original_path(signals).await?;
        let run = crate::ping_cli::run(cli, crate::ping::Stage::Revoked, "b", &path).await?;
        signals
            .write(
                "ping-revoked",
                &Signal::PingRun {
                    run: Box::new(run.clone()),
                },
            )
            .await?;
        ping.as_mut()
            .ok_or("missing source ping evidence")?
            .runs
            .push(run);
        signals.write("revoked-traffic", &Signal::Ready).await?;
    }
    let Signal::Revision { revision } = signals.wait("restored").await? else {
        return Err("invalid restoration rendezvous".into());
    };
    let status = running(cli, revision).await?;
    publish_status(cli, signals, role, "restored", status).await?;
    if let Some(dns) = dns {
        dns.active(
            cli,
            role,
            &dns_domain,
            &addresses[&Role::B],
            "restored",
            true,
        )
        .await?;
        if role == Role::A {
            dns.resolved_restart(cli, role, &dns_domain, &addresses[&Role::B])
                .await?;
        }
    }
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
        let path = crate::fallback::original_path(signals).await?;
        let evidence = ping.as_mut().ok_or("missing source ping evidence")?;
        let sockets_before = traffic.sockets()?;
        let counts_before = traffic.counts;
        evidence
            .runs
            .push(crate::ping_cli::run(cli, crate::ping::Stage::Restored, "b", &path).await?);
        let mut child =
            crate::ping_cli::Live::start(cli, crate::ping::Stage::Interrupted, "b", 1000).await?;
        child.first_sample().await?;
        child.interrupt().await?;
        let run = child.finish(cli, IO).await?;
        run.validate(&path)?;
        evidence.runs.push(run);
        // Same live business sockets must still work after both CLI exit modes.
        traffic.exchange().await?;
        evidence.business_continuity = Some(crate::ping::BusinessContinuity {
            sockets_before,
            sockets_after: traffic.sockets()?,
            counts_before,
            counts_after: traffic.counts,
        });
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
    interface_change::run(cli, role, signals, revision, &mut traffic).await?;
    let pre_outage = running(cli, revision).await?;
    save(cli, "before-outage-status", &pre_outage).await?;
    signals
        .write(&format!("lease-ready-{}", role.name()), &Signal::Ready)
        .await?;
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
    let (recovery, recovered_tun) = reconnect::run(reconnect::Context {
        cli,
        role,
        signals,
        addresses: &addresses,
        dns_domain: &dns_domain,
        revision,
        before: pre_outage,
        old_tun: &tun,
        echo: echo.as_mut().ok_or("missing echo owner")?,
        traffic: &mut traffic,
        dns,
        ping: &mut ping,
    })
    .await?;
    tun = recovered_tun;
    // The real listener/database owner stops only after all recovered packet
    // and State checks. Explicit down is therefore still exercised offline.
    signals.ready("offline").await?;
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
    let dns_evidence = match dns.take() {
        Some(dns) => Some(dns.down(cli, role).await?),
        None => None,
    };
    if let Some(ping) = &ping {
        ping.validate(identity.node_id, &recovery)?;
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
                dns: dns_evidence,
                fallback: fallback_evidence,
                recovery: Box::new(recovery),
                ping: ping.map(Box::new),
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InboundStreamEvidence {
    #[serde(flatten)]
    reception: SubscriptionEvidence,
    start_netmap: meshway_protocol::ipc::NetmapStatus,
    end_netmap: meshway_protocol::ipc::NetmapStatus,
    circuit: String,
}

async fn inbound_stream_evidence(
    cli: &Cli,
    reception: SubscriptionEvidence,
    start_netmap: meshway_protocol::ipc::NetmapStatus,
) -> Result<InboundStreamEvidence> {
    let end_netmap = command(cli, "status")
        .await?
        .netmap
        .ok_or("subscription lost its netmap")?;
    let path = crate::fallback::original_path(&Signals::new(&cli.signals)).await?;
    let mut circuits = std::collections::BTreeSet::new();
    let mut first = None;
    let mut last = None;
    for event in log_events(cli).await? {
        if let Some(packet) = crate::fallback::packet_observation(&event)?
            && packet.observed_at_ms >= reception.start_wall_ms
            && packet.observed_at_ms <= reception.end_wall_ms
        {
            if serde_json::from_str::<Vec<meshway_model::NodeId>>(&packet.path_json)? != path {
                return Err("receive-only stream bypassed its complete path".into());
            }
            circuits.insert(packet.cid);
            first = Some(first.map_or(packet.observed_at_ms, |old: u64| {
                old.min(packet.observed_at_ms)
            }));
            last = Some(last.map_or(packet.observed_at_ms, |old: u64| {
                old.max(packet.observed_at_ms)
            }));
        }
    }
    if reception.sent_initial_count == 0
        || reception.post_start_sent_count != reception.sent_initial_count
        || reception.receive_only_elapsed_ms < cli.traffic_duration().as_millis()
        || reception.max_sequence == 0
        || reception.received_count == 0
        || start_netmap.netmap != end_netmap.netmap
        || circuits.len() != 1
        || first.is_none_or(|at| at > reception.start_wall_ms.saturating_add(2_000))
        || last.is_none_or(|at| at.saturating_add(2_000) < reception.end_wall_ms)
        || end_netmap.expires_at.timestamp_millis() <= i64::try_from(reception.end_wall_ms)?
    {
        return Err(
            "receive-only stream lacks same-socket/circuit/netmap evidence for the complete window"
                .into(),
        );
    }
    Ok(InboundStreamEvidence {
        reception,
        start_netmap,
        end_netmap,
        circuit: circuits
            .into_iter()
            .next()
            .ok_or("missing original circuit")?,
    })
}

/// Known local tracing envelope only. Selected events are subsequently decoded
/// into closed field schemas; this is never used for Server wire responses.
pub(super) async fn log_events(cli: &Cli) -> Result<Vec<serde_json::Value>> {
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut file = tokio::fs::File::open(cli.evidence.join("daemon.stderr"))
        .await?
        .take(LIMIT + 1);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > LIMIT {
        return Err("daemon observation log exceeds evidence bound".into());
    }
    let text = std::str::from_utf8(&bytes)?;
    let end = text
        .rfind('\n')
        .ok_or("daemon observation log has no complete event")?;
    text[..end]
        .lines()
        .map(|line| serde_json::from_str(line).map_err(Into::into))
        .collect()
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

pub(super) async fn wait_ipc(cli: &Cli, daemon: &mut Child) -> Result<()> {
    tokio::time::timeout(crate::remaining(STARTUP)?, async {
        while !tokio::fs::try_exists(cli.state.join("node/control.sock")).await? {
            if daemon.try_wait()?.is_some() {
                return Err("daemon exited before IPC ready".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<(), Error>(())
    })
    .await?
}

pub(super) async fn wait_status(cli: &Cli, predicate: impl Fn(&Status) -> bool) -> Result<Status> {
    wait_status_for(cli, STARTUP, predicate).await
}

async fn wait_status_for(
    cli: &Cli,
    budget: Duration,
    predicate: impl Fn(&Status) -> bool,
) -> Result<Status> {
    let deadline = Instant::now() + crate::remaining(budget)?;
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

pub(super) async fn command(cli: &Cli, operation: &str) -> Result<Status> {
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

pub(super) fn private_file(path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?)
}

async fn save_dns_journals(cli: &Cli) -> Result<()> {
    use std::io::Write;
    let mut directories = tokio::fs::read_dir("/run/meshway").await?;
    let mut remaining = 16;
    while let Some(directory) = directories.next_entry().await? {
        if !directory.file_type().await?.is_dir()
            || !directory.file_name().to_string_lossy().starts_with("dns-")
        {
            continue;
        }
        let mut files = tokio::fs::read_dir(directory.path()).await?;
        while let Some(file) = files.next_entry().await? {
            if remaining == 0 {
                return Ok(());
            }
            if !file.file_type().await?.is_file()
                || file
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let mut input = tokio::fs::File::open(file.path())
                .await?
                .take(256 * 1024 + 1);
            let mut bytes = Vec::new();
            input.read_to_end(&mut bytes).await?;
            if bytes.len() > 256 * 1024 {
                continue;
            }
            let name = format!(
                "failure-{}-{}",
                directory.file_name().to_string_lossy(),
                file.file_name().to_string_lossy()
            );
            private_file(&cli.evidence.join(name))?.write_all(&bytes)?;
            remaining -= 1;
        }
    }
    Ok(())
}

pub(super) async fn terminate(child: &mut Child) -> Result<()> {
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

pub(super) async fn kill_join(child: &mut Child) -> Result<()> {
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
    // Only this actor's namespace; the sole mutation installs the fixture NAT
    // default route before the daemon baseline is taken.
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

pub(super) async fn save(cli: &Cli, name: &str, value: &impl Serialize) -> Result<()> {
    tokio::fs::write(
        cli.evidence.join(format!("{name}.json")),
        serde_json::to_vec_pretty(value)?,
    )
    .await?;
    Ok(())
}
