//! Drive the control owner with bounded retries and cancellable desired-state waits.
mod cycle;

use super::{intent::Desired, network, status::Report};
use crate::settings::Settings;
use cycle::{cycle, failure};
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
) -> Result<(), &'static str> {
    let mut control: Option<Control> = None;
    let mut network: Option<network::Handle> = None;
    let mut queued = None;
    let mut plan = desired.borrow_and_update().clone();
    let mut last_renew: Option<Instant> = None;
    let mut last_report: Option<Instant> = None;
    let mut backoff = 1u64;
    let mut reset = plan.retry_enrollment;
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
        if !plan.up {
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
                    if let Some(request) = queued.take()
                        && let Err(error) = network::request(driver, request).await {
                        tracing::debug!(%error, "Network control intent failed");
                    }
                    cycle(
                    driver,
                    &settings,
                    last_renew.is_none_or(|t| t.elapsed() >= RENEW),
                    last_report.is_none_or(|t| t.elapsed() >= REPORT),
                    &mut reset,
                    network.as_mut(),
                ).await
                } => Some(result),
            };
            match outcome {
                Some(Ok(result)) => {
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
                    match wait_work(&mut desired, &plan, Some(result.delay), network.as_mut()).await
                    {
                        Waiting::Change => {}
                        Waiting::Request(request) => {
                            queued = Some(request);
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
                    let delay = error
                        .retry
                        .then(|| error.retry_after.unwrap_or(Duration::from_secs(backoff)));
                    if !error.retry {
                        stop(driver, false).await;
                        if let Some(network) = &mut network {
                            let _ = network.stop(false).await;
                        }
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
                    match wait_work(&mut desired, &plan, delay, network.as_mut()).await {
                        Waiting::Change => {}
                        Waiting::Request(request) => {
                            queued = Some(request);
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
        if let Some(network) = &mut network {
            let _ = network.stop(false).await;
        }
        queued = None;
        if control.as_ref().is_some_and(|c| c.status().reload_required)
            && let Some(driver) = control.take()
        {
            let _ = driver.shutdown().await;
        }
        plan = desired.borrow_and_update().clone();
        reset = plan.retry_enrollment;
        last_renew = None;
        last_report = None;
        backoff = 1;
    }
    if let Some(mut driver) = control.take() {
        stop(&mut driver, false).await;
        let _ = driver.shutdown().await;
    }
    if let Some(network) = network {
        let keep_sink = desired.borrow().up;
        if let Err(problem) = network.shutdown(keep_sink).await {
            tracing::error!(?problem, "Network shutdown failed");
            return Err("Network shutdown failed.");
        }
    }
    Ok(())
}

enum Waiting {
    Change,
    Timer,
    Worker,
    Request(network::Request),
}
async fn wait_work(
    desired: &mut watch::Receiver<Desired>,
    plan: &Desired,
    delay: Option<Duration>,
    network: Option<&mut network::Handle>,
) -> Waiting {
    tokio::select! { biased;
        _ = desired.wait_for(|next| next.shutdown || next.revision != plan.revision) => Waiting::Change,
        request = async {
            match network { Some(network) => network.requests.recv().await, None => std::future::pending().await }
        } => match request { Some(request) => Waiting::Request(request), None => Waiting::Worker },
        _ = async { match delay { Some(delay) => tokio::time::sleep(delay).await, None => std::future::pending().await } } => Waiting::Timer,
    }
}
