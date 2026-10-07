//! Drive the control owner with bounded retries and cancellable desired-state waits.
mod cycle;
pub(super) mod events;
mod failure;
mod observations;
mod scheduling;

use super::{intent::Desired, network, status::Report};
use crate::settings::Settings;
use cycle::cycle;
use failure::failure;
use meshway_control::{Control, Error};
use meshway_ipc::{Phase, Problem};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

const RENEW: Duration = Duration::from_secs(20);
const REPORT: Duration = Duration::from_secs(20);
const RELEASE: Duration = Duration::from_secs(2);

fn publish(
    sender: &watch::Sender<Report>,
    plan: &Desired,
    control: Option<&Control>,
    phase: Phase,
    url: Option<String>,
    problem: Option<Problem>,
    retry: Option<Duration>,
) {
    let previous = sender.borrow();
    let changed = previous.revision != plan.revision
        || previous.phase != phase
        || previous.problem != problem;
    drop(previous);
    if let Some(problem) = problem {
        tracing::warn!(
            revision = plan.revision.get(),
            ?phase,
            ?problem,
            retry_ms = retry.map(|d| d.as_millis() as u64),
            "Node control problem"
        );
    } else if changed {
        tracing::info!(
            revision = plan.revision.get(),
            ?phase,
            "Node control phase changed"
        );
    }
    sender.send_replace(Report {
        revision: plan.revision,
        phase,
        control: control.map(Control::snapshot),
        enrollment_url: url,
        problem,
        retry_at: retry
            .and_then(|d| chrono::Duration::from_std(d).ok())
            .map(|d| chrono::Utc::now() + d),
        observed_at: chrono::Utc::now(),
        rotation: None,
    });
}

async fn stop(control: &mut Control, cancel_enrollment: bool) {
    // stop invalidates snapshots before its first network wait.
    let _ = tokio::time::timeout(RELEASE, control.stop()).await;
    if cancel_enrollment
        && control.status().enrollment_request_id.is_some()
        && !control.status().reload_required
    {
        let result = tokio::time::timeout(RELEASE, control.cancel_enrollment()).await;
        if matches!(result, Ok(Err(Error::Lifecycle))) && control.can_redeem_enrollment() {
            // Recover an already accepted redemption without going online.
            let _ = tokio::time::timeout(RELEASE, control.redeem_enrollment()).await;
        }
    }
}

async fn wait_change(
    receiver: &mut watch::Receiver<Desired>,
    plan: &Desired,
    delay: Option<Duration>,
) -> bool {
    let changed = receiver.wait_for(|next| next.shutdown || next.revision != plan.revision);
    match delay {
        Some(delay) => tokio::time::timeout(delay, changed).await.is_ok(),
        None => {
            let _ = changed.await;
            true
        }
    }
}

pub(super) async fn run(
    settings: Arc<Settings>,
    mut desired: watch::Receiver<Desired>,
    reported: watch::Sender<Report>,
    directory: Arc<meshway_private_storage::Directory>,
    network_reported: watch::Sender<network::Report>,
    mut pings: tokio::sync::mpsc::Receiver<super::PingSession>,
    event_status: watch::Sender<events::EventStatus>,
) -> Result<(), &'static str> {
    let mut control: Option<Control> = None;
    let mut network: Option<network::Handle> = None;
    let mut queued = None;
    let mut plan = desired.borrow_and_update().clone();
    let mut last_renew: Option<Instant> = None;
    let mut last_report: Option<Instant> = None;
    let mut backoff = 1u64;
    let mut reset = plan.retry_enrollment;
    let mut shutdown_failure = None;
    let mut pending_failure = None;
    let mut observations = observations::Subscriptions::default();
    let mut events = events::Events::new(event_status);
    loop {
        if plan.shutdown || desired.has_changed().is_err() {
            break;
        }
        if control.is_none() {
            if settings.network_mode == meshway_ipc::NetworkMode::Tun && network.is_none() {
                match network::Handle::open(
                    directory.clone(),
                    &settings.server,
                    network_reported.clone(),
                    settings.underlay.clone(),
                    settings.dns.clone(),
                )
                .await
                {
                    Ok(handle) => network = Some(handle),
                    Err(problem) => {
                        publish(
                            &reported,
                            &plan,
                            None,
                            Phase::Blocked,
                            None,
                            Some(problem),
                            None,
                        );
                        wait_change(&mut desired, &plan, None).await;
                        plan = desired.borrow_and_update().clone();
                        reset = plan.retry_enrollment;
                        continue;
                    }
                }
            }
            let builder = match &network {
                Some(network) => match network.client_builder().await {
                    Ok(builder) => builder,
                    Err(problem) => {
                        publish(
                            &reported,
                            &plan,
                            None,
                            Phase::Blocked,
                            None,
                            Some(problem),
                            None,
                        );
                        wait_change(&mut desired, &plan, None).await;
                        plan = desired.borrow_and_update().clone();
                        continue;
                    }
                },
                None => reqwest::Client::builder().no_proxy(),
            };
            let settings = settings.clone();
            let opened = tokio::task::spawn_blocking(move || {
                Control::open(
                    &settings.data_dir.join("identity"),
                    &settings.server,
                    builder,
                )
            })
            .await;
            match opened {
                Ok(Ok(value)) => control = Some(value),
                other => {
                    let problem = match other {
                        Ok(Err(error)) => failure(error, true).problem,
                        _ => Problem::Worker,
                    };
                    publish(
                        &reported,
                        &plan,
                        None,
                        Phase::Blocked,
                        None,
                        Some(problem),
                        None,
                    );
                    wait_change(&mut desired, &plan, None).await;
                    plan = desired.borrow_and_update().clone();
                    reset = plan.retry_enrollment;
                    continue;
                }
            }
            let latest = desired.borrow_and_update().clone();
            if latest.revision != plan.revision || latest.shutdown {
                plan = latest;
                reset = plan.retry_enrollment;
                continue;
            }
        }
        let driver = control.as_mut().expect("control opened above");
        if let Some(id) = plan.rotation {
            events.stop().await;
            observations.stop();
            stop(driver, false).await;
            if let Some(network) = &mut network {
                let _ = network.stop(false).await;
            }
            let result = driver
                .rotate_keys_with_request(id)
                .await
                .map(|_| ())
                .map_err(|error| {
                    let local = match &error {
                        Error::Pending => Some(meshway_ipc::ErrorCode::Pending),
                        Error::Lifecycle => Some(meshway_ipc::ErrorCode::NotReady),
                        _ => None,
                    };
                    let problem = failure::at("rotate_keys", error, true).problem;
                    local.unwrap_or(match problem {
                        Problem::Network => meshway_ipc::ErrorCode::Network,
                        Problem::ServerBusy => meshway_ipc::ErrorCode::ServerBusy,
                        Problem::Credentials => meshway_ipc::ErrorCode::Credentials,
                        Problem::Storage => meshway_ipc::ErrorCode::Storage,
                        Problem::Clock | Problem::Apply => meshway_ipc::ErrorCode::NotReady,
                        _ => meshway_ipc::ErrorCode::Contract,
                    })
                });
            publish(
                &reported,
                &plan,
                Some(driver),
                Phase::Synchronizing,
                None,
                None,
                None,
            );
            reported.send_modify(|report| {
                report.rotation = Some(super::status::RotationOutcome { id, result })
            });
            let _ = desired
                .wait_for(|next| next.shutdown || next.rotation != Some(id))
                .await;
        } else if !plan.up {
            stop(driver, true).await;
            let cleanup = match &mut network {
                Some(network) => network.stop(true).await,
                None => Ok(()),
            };
            publish(
                &reported,
                &plan,
                Some(driver),
                if cleanup.is_ok() {
                    Phase::Down
                } else {
                    Phase::Blocked
                },
                None,
                cleanup.err(),
                None,
            );
            wait_change(&mut desired, &plan, None).await;
        } else {
            if reported.borrow().revision != plan.revision || reported.borrow().phase == Phase::Down
            {
                publish(
                    &reported,
                    &plan,
                    Some(driver),
                    Phase::Starting,
                    None,
                    None,
                    None,
                );
            }
            let outcome = tokio::select! { biased;
                _ = desired.wait_for(|next| next.shutdown || next.revision != plan.revision) => None,
                result = async {
                    if let Some(error) = pending_failure.take() { return Err(error); }
                    let requests_started = Instant::now();
                    while let Ok(ping) = pings.try_recv() { observations.accept(ping, driver); }
                    for _ in 0..32 {
                        let request = queued.take().or_else(|| network.as_mut().and_then(|network| network.requests.try_recv().ok()));
                        let Some(request) = request else { break; };
                        if let Err(error) = network::request(driver, request).await {
                            tracing::debug!(%error, "Network control intent failed");
                        }
                        // Always return to renewal/report work, including under
                        // traversal pressure. Each request has its own deadline.
                        if requests_started.elapsed() >= Duration::from_millis(100) { break; }
                    }
                    observations.advance(driver, network.as_mut()).await;
                    let result = cycle(
                    driver,
                    &settings,
                    last_renew.is_none_or(|t| t.elapsed() >= RENEW),
                    last_report.is_none_or(|t| t.elapsed() >= REPORT),
                    &mut reset,
                    network.as_mut(),
                    plan.enrollment_key.as_ref(),
                ).await?;
                    Ok::<_, failure::Failure>(result)
                } => Some(result),
            };
            match outcome {
                Some(Ok(result)) => {
                    events.advance(driver).await;
                    tracing::debug!(
                        renewed = result.renewed,
                        reported = result.reported,
                        "Node control cycle completed"
                    );
                    if result.renewed {
                        last_renew = Some(Instant::now());
                    }
                    if result.reported {
                        last_report = Some(Instant::now());
                    }
                    backoff = 1;
                    publish(
                        &reported,
                        &plan,
                        Some(driver),
                        result.phase,
                        result.url,
                        None,
                        None,
                    );
                    match wait_work(
                        &mut desired,
                        &plan,
                        Some(result.delay),
                        network.as_mut(),
                        scheduling::reports_enabled(result.phase),
                        Some(driver),
                        &mut pings,
                        &mut observations,
                        &settings,
                        &mut last_report,
                        &mut events,
                    )
                    .await
                    {
                        Waiting::Change => {}
                        Waiting::Failed(error) => {
                            pending_failure = Some(error);
                            continue;
                        }
                        Waiting::Request(request) => {
                            queued = Some(request);
                            continue;
                        }
                        Waiting::Ping(ping) => {
                            observations.accept(ping, driver);
                            continue;
                        }
                        Waiting::Timer => continue,
                        Waiting::Worker => {
                            stop(driver, false).await;
                            publish(
                                &reported,
                                &plan,
                                Some(driver),
                                Phase::Blocked,
                                None,
                                Some(Problem::Worker),
                                None,
                            );
                            wait_change(&mut desired, &plan, None).await;
                        }
                    }
                }
                Some(Err(error)) => {
                    events.stop().await;
                    let delay = error
                        .retry
                        .then(|| error.retry_after.unwrap_or(Duration::from_secs(backoff)));
                    if error.stops_network() {
                        stop(driver, false).await;
                        if let Some(network) = &mut network {
                            let _ = network.stop(false).await;
                        }
                        queued = None;
                        last_renew = None;
                        last_report = None;
                    }
                    publish(
                        &reported,
                        &plan,
                        Some(driver),
                        if error.retry {
                            Phase::Retrying
                        } else {
                            Phase::Blocked
                        },
                        None,
                        Some(error.problem),
                        delay,
                    );
                    backoff = (backoff * 2).min(30);
                    // A fatal identity/contract/storage failure can only be
                    // retried by a new operator revision, never by leftover
                    // network requests waking an otherwise infinite wait.
                    let waiting = if error.retry {
                        wait_work(
                            &mut desired,
                            &plan,
                            delay,
                            network.as_mut(),
                            false,
                            None,
                            &mut pings,
                            &mut observations,
                            &settings,
                            &mut last_report,
                            &mut events,
                        )
                        .await
                    } else {
                        wait_change(&mut desired, &plan, None).await;
                        Waiting::Change
                    };
                    match waiting {
                        Waiting::Change => {}
                        Waiting::Failed(error) => {
                            pending_failure = Some(error);
                            continue;
                        }
                        Waiting::Request(request) => {
                            queued = Some(request);
                            continue;
                        }
                        Waiting::Ping(ping) => {
                            observations.accept(ping, driver);
                            continue;
                        }
                        Waiting::Timer => continue,
                        Waiting::Worker => {
                            stop(driver, false).await;
                            publish(
                                &reported,
                                &plan,
                                Some(driver),
                                Phase::Blocked,
                                None,
                                Some(Problem::Worker),
                                None,
                            );
                            wait_change(&mut desired, &plan, None).await;
                        }
                    }
                }
                None => {}
            }
        }
        stop(control.as_mut().expect("control exists"), false).await;
        events.stop().await;
        observations.stop();
        pending_failure = None;
        if let Some(network) = &mut network {
            let _ = network.stop(false).await;
        }
        queued = None;
        if control.as_ref().is_some_and(|c| c.status().reload_required)
            && let Some(driver) = control.take()
            && let Err(error) = finish_control(driver).await
        {
            shutdown_failure = Some(error);
            break;
        }
        plan = desired.borrow_and_update().clone();
        reset = plan.retry_enrollment;
        last_renew = None;
        last_report = None;
        backoff = 1;
    }
    let keep_sink = desired.borrow().up;
    events.stop().await;
    observations.stop();
    pings.close();
    while let Some(ping) = pings.recv().await {
        let ping = ping.request;
        let _ = ping.events.try_send(meshway_ipc::PingEvent::Failed {
            code: meshway_ipc::PingError::Stopped,
        });
        ping.cancellation.cancel();
    }
    shutdown(
        control,
        async move {
            match network {
                Some(network) => network.shutdown(keep_sink).await,
                None => Ok(()),
            }
        },
        shutdown_failure,
    )
    .await
}

async fn finish_control(mut driver: Control) -> Result<(), &'static str> {
    // The bounded remote release is best-effort and takes the local instance
    // before its first await. shutdown then settles accepted private writes;
    // their final errors must not be treated as remote-release failures.
    stop(&mut driver, false).await;
    driver.shutdown().await.map_err(|error| {
        tracing::error!(%error, "Control private shutdown failed");
        "Control private shutdown failed."
    })
}

async fn shutdown(
    control: Option<Control>,
    network: impl std::future::Future<Output = Result<(), Problem>>,
    mut failure: Option<&'static str>,
) -> Result<(), &'static str> {
    if let Some(driver) = control
        && let Err(error) = finish_control(driver).await
    {
        failure.get_or_insert(error);
    }
    // Always run the resource owner's cleanup and join, even after a failed
    // accepted write. Report both failures without losing the first outcome.
    if let Err(problem) = network.await {
        tracing::error!(?problem, "Network shutdown failed");
        failure.get_or_insert("Network shutdown failed.");
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests;

enum Waiting {
    Failed(failure::Failure),
    Change,
    Timer,
    Worker,
    Request(network::Request),
    Ping(super::PingSession),
}
#[allow(clippy::too_many_arguments)]
async fn wait_work(
    desired: &mut watch::Receiver<Desired>,
    plan: &Desired,
    delay: Option<Duration>,
    mut network: Option<&mut network::Handle>,
    report_changes: bool,
    mut control: Option<&mut Control>,
    pings: &mut tokio::sync::mpsc::Receiver<super::PingSession>,
    observations: &mut observations::Subscriptions,
    settings: &Settings,
    last_report: &mut Option<Instant>,
    events: &mut events::Events,
) -> Waiting {
    let mut window = scheduling::Window::new(delay);
    let mut local_at = Instant::now() + Duration::from_secs(1);
    // A projection invalidation is not an urgent new grant. Keep the previous
    // five-second refresh admission bound under a continuous SSE publication
    // burst; an idle connection can still wake before the periodic fallback.
    let event_refresh_at = Instant::now() + Duration::from_secs(5);
    let mut event_refresh_pending = false;
    let mut event_budget = 32;
    loop {
        if desired.borrow().shutdown
            || desired.borrow().revision != plan.revision
            || desired.has_changed().is_err()
        {
            return Waiting::Change;
        }
        if event_budget == 0 {
            tokio::task::yield_now().await;
            event_budget = 32;
        }
        let waiting = tokio::select! { biased;
            _ = tokio::time::sleep_until(event_refresh_at), if event_refresh_pending => return Waiting::Timer,
            notice = events.next(), if control.is_some() => {
                event_budget -= 1;
                match control.as_deref_mut().expect("event Control").receive_event(notice) {
                    Ok(true) => { event_refresh_pending = true; continue; },
                    Ok(false) => continue,
                    Err(error) => return Waiting::Failed(failure::at("watch_node", error, false)),
                }
            },
            ping = pings.recv() => { if let Some(ping) = ping { return Waiting::Ping(ping); } else { return Waiting::Change; } },
            waiting = window
            .wait(
                async {
                    let _ = desired
                        .wait_for(|next| next.shutdown || next.revision != plan.revision)
                        .await;
                },
                async {
                    match network.as_deref_mut() {
                        Some(network) => {
                            tokio::select! { biased;
                                _ = tokio::time::sleep_until(local_at), if !observations.is_empty() && control.is_some() => None,
                                work = network.work(report_changes) => Some(work),
                            }
                        },
                        None => std::future::pending().await,
                    }
                },
            )
            => waiting,
        };
        let waiting = match waiting {
            scheduling::Wake::Changed => Waiting::Change,
            scheduling::Wake::Due => Waiting::Timer,
            scheduling::Wake::Work(work) => match work {
                Some(network::Work::Request(request)) => Waiting::Request(request),
                Some(network::Work::Report) => {
                    let ready = control
                        .as_deref()
                        .zip(network.as_deref())
                        .is_some_and(|(control, network)| network.can_report_inline(control));
                    let result = window
                        .report(
                            ready,
                            async {
                                let _ = desired
                                    .wait_for(|next| {
                                        next.shutdown || next.revision != plan.revision
                                    })
                                    .await;
                            },
                            network
                                .as_deref_mut()
                                .expect("current network")
                                .report_state(
                                    control.as_deref_mut().expect("current Control"),
                                    settings,
                                    false,
                                ),
                        )
                        .await;
                    match result {
                        scheduling::Wake::Changed => return Waiting::Change,
                        scheduling::Wake::Work(scheduling::Report::Refresh) => {
                            return Waiting::Timer;
                        }
                        scheduling::Wake::Work(scheduling::Report::Sent(Ok(fresh))) => {
                            if fresh {
                                *last_report = Some(Instant::now());
                            }
                            continue;
                        }
                        scheduling::Wake::Work(scheduling::Report::Sent(Err(error))) => {
                            return Waiting::Failed(failure::at(
                                "report_runtime_state",
                                error,
                                false,
                            ));
                        }
                        scheduling::Wake::Due => {
                            unreachable!("admitted State report retains its deadline")
                        }
                    }
                }
                Some(network::Work::Closed) => Waiting::Worker,
                None => {
                    local_at = Instant::now() + Duration::from_secs(1);
                    match window
                        .serve(
                            async {
                                let _ = desired
                                    .wait_for(|next| {
                                        next.shutdown || next.revision != plan.revision
                                    })
                                    .await;
                            },
                            observations.advance(
                                control.as_deref_mut().expect("maintenance has Control"),
                                network.as_deref_mut(),
                            ),
                        )
                        .await
                    {
                        scheduling::Wake::Changed => return Waiting::Change,
                        scheduling::Wake::Work(true) => return Waiting::Timer,
                        scheduling::Wake::Work(false) => continue,
                        scheduling::Wake::Due => {
                            unreachable!("admitted maintenance retains its deadline")
                        }
                    }
                }
            },
        };
        match waiting {
            Waiting::Request(request) if request.inline() && control.is_some() => {
                let result = window
                    .serve(
                        async {
                            let _ = desired
                                .wait_for(|next| next.shutdown || next.revision != plan.revision)
                                .await;
                        },
                        network::request(
                            control.as_deref_mut().expect("Control owner present"),
                            request,
                        ),
                    )
                    .await;
                let result = match result {
                    scheduling::Wake::Changed => return Waiting::Change,
                    scheduling::Wake::Due => {
                        unreachable!("an admitted RPC retains its original deadline")
                    }
                    scheduling::Wake::Work(result) => result,
                };
                if let Err(error) = result {
                    tracing::debug!(%error, "Acquisition control intent failed");
                    return Waiting::Timer;
                }
            }
            waiting => return waiting,
        }
    }
}
