//! One shared outage/recovery stage for every carrier, DNS backend and NAT mode.
use super::*;
use crate::recovery::{Evidence, Expired, now_ms};

pub(super) struct Context<'a> {
    pub cli: &'a Cli,
    pub role: Role,
    pub signals: &'a Signals,
    pub addresses: &'a BTreeMap<Role, Addresses>,
    pub dns_domain: &'a str,
    pub revision: Revision,
    pub before: Status,
    pub old_tun: &'a str,
    pub echo: &'a mut Echo,
    pub traffic: &'a mut Option<Traffic>,
    pub dns: &'a mut Option<crate::dns::Probe>,
    pub ping: &'a mut Option<crate::ping::PingEvidence>,
}

pub(super) async fn run(context: Context<'_>) -> Result<(Evidence, String)> {
    let mut live = if context.role == Role::A {
        let mut live =
            crate::ping_cli::Live::start(context.cli, crate::ping::Stage::Outage, "b", 1000)
                .await?;
        live.first_sample().await?;
        Some(live)
    } else {
        // Only A exercises restored business, finite ping and SIGINT before
        // reaching this stage. Use the same existing stage budget as Controller
        // instead of starting the HTTP-outage timer while those checks run.
        if !matches!(
            context
                .signals
                .wait_for("lease-ready-a", STARTUP + crate::ping::STAGE_LIMIT * 2,)
                .await?,
            Signal::Ready
        ) {
            return Err("invalid restored traffic/CLI lifecycle completion".into());
        }
        context.signals.ready("ping-outage-ready").await?;
        None
    };
    let result = exercise(context, &mut live).await;
    let cleanup = match live {
        Some(live) => live.abort().await,
        None => Ok(()),
    };
    result.and_then(|value| cleanup.map(|()| value))
}

async fn exercise(
    context: Context<'_>,
    live: &mut Option<crate::ping_cli::Live>,
) -> Result<(Evidence, String)> {
    let Context {
        cli,
        role,
        signals,
        addresses,
        dns_domain,
        revision,
        before,
        old_tun,
        echo,
        traffic,
        dns,
        ping,
    } = context;
    if role == Role::A {
        signals.write("ping-outage-ready", &Signal::Ready).await?;
    }
    let Signal::HttpAvailability {
        available: false,
        observed_at_ms: closed_at_ms,
    } = signals.wait("http-outage").await?
    else {
        return Err("invalid HTTP outage boundary".into());
    };
    let fresh = command(cli, "status").await?;
    let lease = before.instance.as_ref().ok_or("no pre-outage lease")?;
    let (mut instance_expiry, execution_expiry) = crate::recovery::outage_deadlines(
        lease,
        fresh.instance.as_ref(),
        fresh
            .netmap
            .as_ref()
            .or(before.netmap.as_ref())
            .ok_or("missing pre-outage netmap lease")?,
    )?;
    if instance_expiry <= closed_at_ms || instance_expiry.saturating_sub(closed_at_ms) > 130_000 {
        return Err("unexpected actual instance deadline at HTTP outage".into());
    }
    save(cli, "lease-at-outage", &fresh).await?;
    tracing::info!(
        phase = "outage-deadlines",
        role = role.name(),
        instance_expiry,
        execution_expiry,
        current_execution_observed = fresh.netmap.is_some(),
        "Retained observed deadlines across the HTTP boundary"
    );
    let old_sockets = traffic.as_ref().map(Traffic::sockets).transpose()?;
    let path = crate::fallback::original_path(signals).await?;
    let old_circuits = if role == Role::A {
        crate::recovery::circuit_ids(cli, &path, None).await?
    } else {
        Vec::new()
    };
    let wait = Duration::from_millis(execution_expiry.saturating_sub(now_ms()?) + 500);
    if crate::remaining(wait)? < wait {
        return Err("execution expiry exceeds fixture deadline".into());
    }
    tokio::time::sleep(wait).await;
    // A publication may have hidden its successor's execution deadline while
    // retaining the instance. Observe actual suspension within the independently
    // bounded instance horizon, rather than inventing an execution lease.
    let suspension_budget = Duration::from_millis(
        closed_at_ms
            .saturating_add(130_000)
            .saturating_sub(now_ms()?),
    );
    let suspended = wait_status_for(cli, suspension_budget, |status| {
        status.data_plane == DataPlane::Suspended && status.desired_up
    })
    .await?;
    save(cli, "lease-expired-status", &suspended).await?;
    if let Some(dns) = dns.as_mut() {
        dns.expired(cli, role, dns_domain).await?;
    }
    route_proof(cli, &addresses[&Role::B], old_tun, true, "suspended").await?;
    let old_traffic_blocked = if let Some(traffic) = traffic.as_mut() {
        tracing::info!(
            phase = "expired-business-block",
            role = role.name(),
            "Checking old Mesh business sockets"
        );
        traffic.blocked().await?;
        Some(true)
    } else {
        None
    };
    application::underlay_probe(signals::underlay(cli).await?.actors[&Role::B]).await?;
    tracing::info!(
        phase = "expired-echo-check",
        role = role.name(),
        "Checking existing application workers"
    );
    echo.check()?;

    // Keep sampling real status during the closed gate: an admitted refresh
    // just before closure may have supplied a later lease than the first read.
    let deadline = Instant::now() + crate::remaining(Duration::from_secs(140))?;
    let expired_status = loop {
        let status = command(cli, "status").await?;
        if status.identity != before.identity
            || status.process_id != before.process_id
            || status.revision != before.revision
            || !status.desired_up
            || status.phase == Phase::Blocked
        {
            return Err(
                "daemon changed identity/intent or became permanently blocked during outage".into(),
            );
        }
        if let Some(lease) = &status.instance {
            instance_expiry = instance_expiry.max(lease.expires_at_ms);
        }
        if instance_expiry.saturating_sub(closed_at_ms) > 130_000 {
            return Err("Node instance was renewed beyond the drained HTTP gate".into());
        }
        if status.instance.is_none() && status.applied.is_none() && now_ms()? > instance_expiry {
            break status;
        }
        if Instant::now() >= deadline {
            return Err("actual instance did not expire during HTTP outage".into());
        }
        echo.check()?;
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let expired = Expired {
        before: Box::new(before),
        status: Box::new(expired_status),
        closed_at_ms,
        instance_expires_at_ms: instance_expiry,
        observed_at_ms: now_ms()?,
        old_traffic_blocked,
    };
    if let Some(live) = live.take() {
        let run = live.finish(cli, crate::IO).await?;
        run.validate(&path)?;
        if run.samples().any(|sample| {
            sample.probe.as_ref().is_some_and(|probe| {
                probe.outcome == meshway_model::state::ProbeOutcome::Success
                    && probe.completed_at.is_none_or(|at| {
                        at.timestamp_millis() > i64::try_from(instance_expiry).unwrap_or(i64::MAX)
                    })
            })
        }) {
            return Err("old CLI emitted a successful measurement after instance expiry".into());
        }
        let evidence = ping.as_mut().ok_or("source ping evidence missing")?;
        evidence.runs.push(run);
        evidence
            .runs
            .push(crate::ping_cli::run(cli, crate::ping::Stage::Expired, "b", &path).await?);
    }
    expired.validate()?;
    save(cli, "instance-expired", &expired).await?;
    signals
        .write(
            &format!("instance-expired-{}", role.name()),
            &Signal::Expired {
                role,
                evidence: Box::new(expired.clone()),
            },
        )
        .await?;
    let Signal::HttpAvailability {
        available: true,
        observed_at_ms: reopened_at_ms,
    } = signals
        .wait_for("recovery-open", Duration::from_secs(75))
        .await?
    else {
        return Err("invalid HTTP recovery boundary".into());
    };
    let recovered = tokio::time::timeout(crate::remaining(Duration::from_secs(75))?, async {
        loop {
            let status = command(cli, "status").await?;
            if status.identity != expired.before.identity
                || status.process_id != expired.before.process_id
                || status.revision != expired.before.revision
                || !status.desired_up
                || status.phase == Phase::Blocked
            {
                return Err(
                    "automatic recovery requires operator intervention or changed daemon identity"
                        .into(),
                );
            }
            if expired.validate_recovered(&status, reopened_at_ms).is_ok() {
                return Ok::<_, Error>(status);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await??;
    publish_status(cli, signals, role, "recovery-applied", recovered).await?;
    if !matches!(
        signals
            .wait_for("recovery-state-verified", Duration::from_secs(120))
            .await?,
        Signal::Ready
    ) {
        return Err("invalid recovered State acknowledgement".into());
    }
    let tun = owned_tun().await?;
    let index = tun_index(&tun).await?;
    let initial_rx = tun_received(&tun).await?;
    let before_counts = echo.counts();
    route_proof(
        cli,
        &addresses[&Role::B],
        &tun,
        role == Role::R,
        "recovered",
    )
    .await?;
    if let Some(dns) = dns.as_mut() {
        dns.active(
            cli,
            role,
            dns_domain,
            &addresses[&Role::B],
            "recovered",
            true,
        )
        .await?;
    }
    signals
        .write(&format!("recovery-ready-{}", role.name()), &Signal::Ready)
        .await?;
    for actor in Role::ALL {
        if !matches!(
            signals
                .wait_for(
                    &format!("recovery-ready-{}", actor.name()),
                    Duration::from_secs(75)
                )
                .await?,
            Signal::Ready
        ) {
            return Err("invalid recovered endpoint readiness".into());
        }
    }
    let started = Instant::now();
    if role == Role::A {
        let run = crate::ping_cli::run(cli, crate::ping::Stage::Recovered, "b", &path).await?;
        ping.as_mut()
            .ok_or("source ping evidence missing")?
            .runs
            .push(run);
    }
    let mut old_tcp_blocked = None;
    let mut packet = None;
    let mut new_circuits = Vec::new();
    if let Some(old) = traffic.as_mut() {
        let fresh = tokio::time::timeout(crate::remaining(Duration::from_secs(100))?, async {
            old.tcp_blocked().await?;
            old_tcp_blocked = Some(true);
            // Keep old FDs alive while allocating new tuples. Expired TCP streams
            // are never reconnected by Traffic::restored or counted as continuity.
            let mut fresh = Traffic::new(&addresses[&Role::A], &addresses[&Role::B]).await?;
            fresh.first_udp().await?;
            fresh.add_demands(&addresses[&Role::A]).await?;
            fresh.exchange().await?;
            Ok::<_, Error>(fresh)
        })
        .await??;
        packet = Some(fresh.evidence(started.elapsed())?);
        new_circuits = crate::recovery::circuit_ids(cli, &path, Some(reopened_at_ms)).await?;
        *traffic = Some(fresh);
        signals.write("recovery-traffic", &Signal::Ready).await?;
    } else if !matches!(
        signals
            .wait_for(
                "recovery-traffic",
                Duration::from_secs(110) + crate::ping::STAGE_LIMIT
            )
            .await?,
        Signal::Ready
    ) {
        return Err("invalid recovered packet rendezvous".into());
    }
    echo.check()?;
    verify_tun_index(&tun, index).await?;
    let received = tun_received(&tun)
        .await?
        .checked_sub(initial_rx)
        .ok_or("recovered TUN counter regressed")?;
    let recovered = running(cli, revision).await?;
    let evidence = Evidence {
        expired,
        recovered: Box::new(recovered),
        reopened_at_ms,
        old_tcp_blocked_after_recovery: old_tcp_blocked,
        old_sockets,
        old_circuits,
        new_circuits,
        traffic: packet,
        delivered: echo.counts().since(before_counts)?,
        relay_tun_received: (role == Role::R).then_some(received),
    };
    evidence.validate(role)?;
    save(cli, "automatic-recovery", &evidence).await?;
    signals
        .write(&format!("recovery-done-{}", role.name()), &Signal::Ready)
        .await?;
    Ok((evidence, tun))
}
