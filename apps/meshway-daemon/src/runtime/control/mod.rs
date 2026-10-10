//! Drive the control owner. One `Control` sits behind a mutex: the `sync`
//! long poll (`meshway_control::sync_loop`) holds it only to prepare and apply
//! each exchange, while this task uses it for enrollment, instance, State,
//! receipts, observation tasks and coordination requests. Every verified
//! netmap the poll publishes is offered to the network owner's Engine; a held
//! signed netmap is offered at once on start, before any Server contact.
mod cycle;
mod failure;
#[cfg(all(test, target_os = "linux"))]
mod tests;

use super::{intent::Desired, network, status::Report};
use crate::settings::Settings;
use cycle::{Online, online};
use failure::failure;
use meshway_control::{Control, Error};
use meshway_ipc::{Phase, Problem};
use meshway_model::{InstanceId, NetmapRef};
use std::{sync::Arc, time::Duration};
use tokio::{
    sync::{Mutex, watch},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

const RENEW: Duration = Duration::from_secs(20);
const REPORT: Duration = Duration::from_secs(20);
const TASKS: Duration = Duration::from_secs(2);
const RELEASE: Duration = Duration::from_secs(2);
/// A sync that has not succeeded for this long is reported as reconnecting:
/// above one Server hold plus the request budget's margin.
pub(super) const SYNC_STALE: Duration = Duration::from_secs(100);

type Shared = Arc<Mutex<Control>>;

/// The `sync` long poll as seen by status: whether it runs, and when it last
/// returned a verified netmap.
#[derive(Clone, Debug, Default)]
pub(crate) struct SyncStatus {
    pub running: bool,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_sync: Option<std::time::Instant>,
}

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

/// Offer Control's verified netmap, packet keys and instance binding to the
/// network owner's Engine. Without a published lease Engine withdraws.
fn offer(control: &Control, network: Option<&network::Handle>) {
    let Some(network) = network else {
        return;
    };
    network.offer(network::Offer {
        netmap: control.netmap(),
        keys: control.dataplane_keys().ok(),
        instance: control
            .status()
            .instance
            .map(|instance| meshway_protocol::InstanceRequest {
                instance_id: instance.instance_id,
                generation: instance.generation,
            }),
    });
}

/// End of one up session.
enum Ended {
    /// The operator intent changed (or shutdown).
    Changed,
    /// A failure that only a new operator revision can retry.
    Blocked(Problem),
    /// The network worker stopped.
    Worker,
}

/// Per-session state of the periodic Control work.
#[derive(Default)]
struct Maintenance {
    renewed: Option<Instant>,
    reported: Option<Instant>,
    tasks: Option<Instant>,
    retry_at: Option<Instant>,
    /// The instance and netmap whose installation outcome was receipted.
    receipted: Option<(InstanceId, NetmapRef, bool)>,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    settings: Arc<Settings>,
    mut desired: watch::Receiver<Desired>,
    reported: watch::Sender<Report>,
    directory: Arc<meshway_private_storage::Directory>,
    network_reported: watch::Sender<network::Report>,
    local_pings: watch::Sender<Option<network::Pings>>,
    sync_status: watch::Sender<SyncStatus>,
) -> Result<(), &'static str> {
    let mut control: Option<Shared> = None;
    let mut network: Option<network::Handle> = None;
    let mut plan = desired.borrow_and_update().clone();
    let mut backoff = 1u64;
    let mut reset = plan.retry_enrollment;
    let mut shutdown_failure = None;
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
                    Ok(handle) => {
                        local_pings.send_replace(Some(handle.pings()));
                        network = Some(handle);
                    }
                    Err(problem) => {
                        let delay =
                            (problem == Problem::Underlay).then_some(Duration::from_secs(backoff));
                        publish(
                            &reported,
                            &plan,
                            None,
                            if delay.is_some() {
                                Phase::Retrying
                            } else {
                                Phase::Blocked
                            },
                            None,
                            Some(problem),
                            delay,
                        );
                        backoff = (backoff * 2).min(30);
                        wait_change(&mut desired, &plan, delay).await;
                        plan = desired.borrow_and_update().clone();
                        reset = plan.retry_enrollment;
                        continue;
                    }
                }
            }
            let builder = match &network {
                Some(network) => network.client_builder().await,
                None => meshway_control_client::http_client_builder()
                    .map(|builder| builder.no_proxy())
                    .map_err(|_| Problem::Contract),
            };
            let builder = match builder {
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
            };
            let opened = {
                let settings = settings.clone();
                tokio::task::spawn_blocking(move || {
                    Control::open(
                        &settings.data_dir.join("identity"),
                        &settings.server,
                        builder,
                    )
                })
                .await
            };
            match opened {
                Ok(Ok(value)) => control = Some(Arc::new(Mutex::new(value))),
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
        let shared = control.clone().expect("control opened above");
        if let Some(id) = plan.rotation {
            let mut driver = shared.lock().await;
            stop(&mut driver, false).await;
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
                Some(&driver),
                Phase::Synchronizing,
                None,
                None,
                None,
            );
            drop(driver);
            reported.send_modify(|report| {
                report.rotation = Some(super::status::RotationOutcome { id, result })
            });
            let _ = desired
                .wait_for(|next| next.shutdown || next.rotation != Some(id))
                .await;
        } else if !plan.up {
            let mut driver = shared.lock().await;
            stop(&mut driver, true).await;
            let cleanup = match &mut network {
                Some(network) => network.stop(true).await,
                None => Ok(()),
            };
            publish(
                &reported,
                &plan,
                Some(&driver),
                if cleanup.is_ok() {
                    Phase::Down
                } else {
                    Phase::Blocked
                },
                None,
                cleanup.err(),
                None,
            );
            drop(driver);
            wait_change(&mut desired, &plan, None).await;
        } else {
            let ended = session(
                &shared,
                network.as_mut(),
                &settings,
                &mut desired,
                &plan,
                &reported,
                &sync_status,
                &mut reset,
            )
            .await;
            sync_status.send_modify(|status| status.running = false);
            match ended {
                Ended::Changed => {}
                Ended::Blocked(problem) => {
                    let mut driver = shared.lock().await;
                    stop(&mut driver, false).await;
                    if let Some(network) = &mut network {
                        let _ = network.stop(false).await;
                    }
                    publish(
                        &reported,
                        &plan,
                        Some(&driver),
                        Phase::Blocked,
                        None,
                        Some(problem),
                        None,
                    );
                    drop(driver);
                    wait_change(&mut desired, &plan, None).await;
                }
                Ended::Worker => {
                    let mut driver = shared.lock().await;
                    stop(&mut driver, false).await;
                    publish(
                        &reported,
                        &plan,
                        Some(&driver),
                        Phase::Blocked,
                        None,
                        Some(Problem::Worker),
                        None,
                    );
                    drop(driver);
                    wait_change(&mut desired, &plan, None).await;
                }
            }
        }
        {
            let mut driver = shared.lock().await;
            stop(&mut driver, false).await;
        }
        if let Some(network) = &mut network {
            let _ = network.stop(false).await;
        }
        let reload = shared.lock().await.status().reload_required;
        if reload {
            drop(shared);
            if let Some(driver) = control.take()
                && let Err(error) = finish_control(driver).await
            {
                shutdown_failure = Some(error);
                break;
            }
        }
        plan = desired.borrow_and_update().clone();
        reset = plan.retry_enrollment;
        backoff = 1;
    }
    let keep_sink = desired.borrow().up;
    local_pings.send_replace(None);
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

/// The operator intent moved on from `plan` (or shutdown).
async fn changed(desired: &mut watch::Receiver<Desired>, plan: &Desired) {
    let _ = desired
        .wait_for(|next| next.shutdown || next.revision != plan.revision)
        .await;
}

/// What woke an idle wait.
enum Wake {
    Changed,
    Due,
    Worker,
}

/// Wait up to `delay` while serving network requests; the
/// data plane keeps running on its installed netmap meanwhile.
async fn idle(
    shared: &Shared,
    mut network: Option<&mut network::Handle>,
    desired: &mut watch::Receiver<Desired>,
    plan: &Desired,
    delay: Duration,
) -> Wake {
    let deadline = Instant::now() + delay;
    loop {
        tokio::select! { biased;
            _ = changed(desired, plan) => return Wake::Changed,
            _ = tokio::time::sleep_until(deadline) => return Wake::Due,
            work = async { match network.as_deref_mut() { Some(network) => network.work().await, None => std::future::pending().await } } => match work {
                network::Work::Request(request) => {
                    let mut driver = shared.lock().await;
                    if let Err(error) = network::request(&mut driver, request).await {
                        tracing::debug!(%error, "Network control intent failed");
                    }
                }
                network::Work::Report => {
                    if let Some(network) = network.as_deref_mut() {
                        network.defer_report();
                    }
                }
                network::Work::Closed => return Wake::Worker,
            },
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn session(
    shared: &Shared,
    mut network: Option<&mut network::Handle>,
    settings: &Settings,
    desired: &mut watch::Receiver<Desired>,
    plan: &Desired,
    reported: &watch::Sender<Report>,
    sync_status: &watch::Sender<SyncStatus>,
    reset: &mut bool,
) -> Ended {
    {
        let driver = shared.lock().await;
        if reported.borrow().revision != plan.revision || reported.borrow().phase == Phase::Down {
            publish(
                reported,
                plan,
                Some(&driver),
                Phase::Starting,
                None,
                None,
                None,
            );
        }
        // A verified held netmap is authority again without Server contact.
        offer(&driver, network.as_deref());
    }
    let mut backoff = 1u64;
    loop {
        // Enrollment and authentication.
        let result = {
            let mut driver = shared.lock().await;
            let result = tokio::select! { biased;
                _ = changed(desired, plan) => return Ended::Changed,
                result = online(&mut driver, settings, reset, plan.enrollment_key.as_ref()) => result,
            };
            match &result {
                Ok(Online::Awaiting { url }) => publish(
                    reported,
                    plan,
                    Some(&driver),
                    Phase::AwaitingApproval,
                    url.clone(),
                    None,
                    None,
                ),
                Ok(Online::Ready) => publish(
                    reported,
                    plan,
                    Some(&driver),
                    Phase::ControlReady,
                    None,
                    None,
                    None,
                ),
                Ok(Online::Restart) => {}
                Err(error) => {
                    let delay = error
                        .retry
                        .then(|| error.retry_after.unwrap_or(Duration::from_secs(backoff)));
                    publish(
                        reported,
                        plan,
                        Some(&driver),
                        if error.retry {
                            Phase::Retrying
                        } else {
                            Phase::Blocked
                        },
                        None,
                        Some(error.problem),
                        delay,
                    );
                    offer(&driver, network.as_deref());
                }
            }
            result
        };
        let delay = match result {
            Ok(Online::Ready) => {
                backoff = 1;
                match online_session(
                    shared,
                    network.as_deref_mut(),
                    settings,
                    desired,
                    plan,
                    reported,
                    sync_status,
                )
                .await
                {
                    Ok(()) => continue,
                    Err(Some(ended)) => return ended,
                    Err(None) => {
                        let delay = Duration::from_secs(backoff);
                        backoff = (backoff * 2).min(30);
                        delay
                    }
                }
            }
            Ok(Online::Awaiting { .. }) => Duration::from_secs(2),
            Ok(Online::Restart) => continue,
            Err(error) if !error.retry => return Ended::Blocked(error.problem),
            Err(error) => {
                let delay = error.retry_after.unwrap_or(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(30);
                delay
            }
        };
        match idle(shared, network.as_deref_mut(), desired, plan, delay).await {
            Wake::Changed => return Ended::Changed,
            Wake::Worker => return Ended::Worker,
            Wake::Due => {}
        }
    }
}

/// The online part of a session: the sync long poll plus periodic Control
/// work. `Ok` re-enters authentication at once; `Err(None)` after a backoff.
#[allow(clippy::too_many_arguments)]
async fn online_session(
    shared: &Shared,
    mut network: Option<&mut network::Handle>,
    settings: &Settings,
    desired: &mut watch::Receiver<Desired>,
    plan: &Desired,
    reported: &watch::Sender<Report>,
    sync_status: &watch::Sender<SyncStatus>,
) -> Result<(), Option<Ended>> {
    let stop = CancellationToken::new();
    let (netmaps, mut netmap_rx) = watch::channel::<Option<meshway_engine::Netmap>>(None);
    netmap_rx.borrow_and_update();
    let standalone = network::Peers::default();
    let mut sink = match network.as_deref() {
        Some(network) => network.peer_sink(),
        None => standalone.sink(),
    };
    let mut sync = {
        let shared = shared.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            meshway_control::sync_loop(&shared, &mut sink, &netmaps, &stop).await
        })
    };
    sync_status.send_modify(|status| status.running = true);
    let mut work = Maintenance::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! { biased;
            _ = changed(desired, plan) => break Err(Some(Ended::Changed)),
            joined = &mut sync => {
                let error = match joined {
                    Ok(Ok(())) => break Ok(()),
                    Ok(Err(error)) => error,
                    Err(_) => break Err(Some(Ended::Worker)),
                };
                let failure = failure::at("sync", error, false);
                let driver = shared.lock().await;
                offer(&driver, network.as_deref());
                if !failure.retry {
                    break Err(Some(Ended::Blocked(failure.problem)));
                }
                publish(reported, plan, Some(&driver), Phase::Retrying, None, Some(failure.problem), None);
                break Err(None);
            },
            changed = netmap_rx.changed() => {
                if changed.is_err() {
                    continue;
                }
                let synced = netmap_rx.borrow_and_update().is_some();
                let driver = shared.lock().await;
                if synced {
                    sync_status.send_modify(|status| {
                        status.last_sync_at = Some(chrono::Utc::now());
                        status.last_sync = Some(std::time::Instant::now());
                    });
                    // Assigned observation tasks may have woken the hold.
                    work.tasks = None;
                }
                offer(&driver, network.as_deref());
                reported.send_modify(|report| report.control = Some(driver.snapshot()));
            },
            next = async { match network.as_deref_mut() { Some(network) => network.work().await, None => std::future::pending().await } } => match next {
                network::Work::Request(request) => {
                    let mut driver = shared.lock().await;
                    if let Err(error) = network::request(&mut driver, request).await {
                        tracing::debug!(%error, "Network control intent failed");
                    }
                },
                network::Work::Report => {
                    let mut driver = shared.lock().await;
                    if let Err(ended) = maintain(&mut driver, network.as_deref_mut(), settings, &mut work, plan, reported, true).await {
                        break Err(Some(ended));
                    }
                    // Pace the change schedule even when nothing was sent.
                    if let Some(network) = network.as_deref_mut() {
                        network.defer_report();
                    }
                },
                network::Work::Closed => break Err(Some(Ended::Worker)),
            },
            _ = tick.tick() => {
                let mut driver = shared.lock().await;
                if let Err(ended) = maintain(&mut driver, network.as_deref_mut(), settings, &mut work, plan, reported, false).await {
                    break Err(Some(ended));
                }
            },
        }
    };
    stop.cancel();
    if !sync.is_finished() {
        let _ = sync.await;
    }
    result
}

/// Periodic instance, State, receipt and observation-task work. Transient
/// failures are retried after a backoff and never stop the data plane;
/// identity and binding rejections block.
#[allow(clippy::too_many_arguments)]
async fn maintain(
    control: &mut Control,
    mut network: Option<&mut network::Handle>,
    settings: &Settings,
    work: &mut Maintenance,
    plan: &Desired,
    reported: &watch::Sender<Report>,
    changed: bool,
) -> Result<(), Ended> {
    let now = Instant::now();
    if work.retry_at.is_some_and(|at| now < at) {
        return Ok(());
    }
    let result = maintain_once(control, network.as_deref_mut(), settings, work, changed).await;
    match result {
        Ok(()) => {
            work.retry_at = None;
            if reported.borrow().problem.is_some() {
                publish(
                    reported,
                    plan,
                    Some(control),
                    Phase::ControlReady,
                    None,
                    None,
                    None,
                );
            } else {
                reported.send_modify(|report| report.control = Some(control.snapshot()));
            }
            Ok(())
        }
        Err((operation, Error::StateScopeChanged)) => {
            tracing::debug!(operation, "State scope rotated; resampling after sync");
            work.retry_at = Some(now + Duration::from_secs(1));
            Ok(())
        }
        Err((operation, error)) => {
            let failure = failure::at(operation, error, operation == "authenticate");
            offer(control, network.as_deref());
            if !failure.retry {
                return Err(Ended::Blocked(failure.problem));
            }
            // A reset context (session or instance gone) is re-acquired at once.
            let delay = failure
                .retry_after
                .unwrap_or(Duration::from_secs(if failure.reset { 1 } else { 5 }));
            work.retry_at = Some(now + delay);
            publish(
                reported,
                plan,
                Some(control),
                Phase::Retrying,
                None,
                Some(failure.problem),
                Some(delay),
            );
            Ok(())
        }
    }
}

async fn maintain_once(
    control: &mut Control,
    network: Option<&mut network::Handle>,
    settings: &Settings,
    work: &mut Maintenance,
    changed: bool,
) -> Result<(), (&'static str, Error)> {
    let now = Instant::now();
    control
        .authenticate()
        .await
        .map_err(|error| ("authenticate", error))?;
    let mut fresh = false;
    if control.status().instance.is_none() {
        control
            .acquire_instance(if network.is_some() {
                network::capabilities()
            } else {
                Default::default()
            })
            .await
            .map_err(|error| ("acquire_instance", error))?;
        work.renewed = Some(now);
        work.receipted = None;
        fresh = true;
        // State/receipt binding of the new instance; nothing reinstalls.
        offer(control, network.as_deref());
    } else if work.renewed.is_none_or(|at| now >= at + RENEW) {
        control
            .renew_instance()
            .await
            .map_err(|error| ("renew_instance", error))?;
        work.renewed = Some(now);
    }
    if let Some(network) = network {
        receipt(control, network, work).await?;
        let routine = fresh || work.reported.is_none_or(|at| now >= at + REPORT);
        if (routine || changed)
            && network
                .report_state(control, settings, routine)
                .await
                .map_err(|error| ("report_runtime_state", error))?
        {
            work.reported = Some(now);
        }
        if work.tasks.is_none_or(|at| now >= at + TASKS)
            && let Some(netmap) = control.netmap_lease().map(|lease| lease.netmap)
        {
            work.tasks = Some(now);
            let tasks = control
                .get_observation_tasks(netmap)
                .await
                .map_err(|error| ("get_observation_tasks", error))?;
            network.observation_tasks(netmap, tasks.items);
        }
    } else if fresh || work.reported.is_none_or(|at| now >= at + REPORT) {
        match control.report_control_state(settings.system.clone()).await {
            Ok(_) => work.reported = Some(now),
            Err(Error::Pending) => {
                control
                    .retry_state_report()
                    .await
                    .map_err(|error| ("retry_state_report", error))?;
            }
            Err(error) => return Err(("report_control_state", error)),
        }
    }
    Ok(())
}

/// Receipt the installation outcome of the installed netmap once per
/// instance: saved, then applied or failed. A new instance receipts the same,
/// already installed netmap again as a new attempt.
async fn receipt(
    control: &mut Control,
    network: &mut network::Handle,
    work: &mut Maintenance,
) -> Result<(), (&'static str, Error)> {
    let report = network.report.borrow().clone();
    let (Some(netmap), Some(completion)) = (report.netmap, report.completion) else {
        return Ok(());
    };
    let Some(instance) = control.status().instance.map(|i| i.instance_id) else {
        return Ok(());
    };
    if !report.current() && completion.is_ok() {
        return Ok(());
    }
    let key = (instance, netmap, completion.is_ok());
    if work.receipted == Some(key) || control.netmap_reference() != Some(netmap) {
        return Ok(());
    }
    let ticket = control
        .begin_application(netmap)
        .await
        .map_err(|error| ("begin_application", error))?;
    control
        .finish_application(&ticket, completion, report.observed_at)
        .await
        .map_err(|error| ("finish_application", error))?;
    work.receipted = Some(key);
    network.receipt_completed();
    Ok(())
}

async fn finish_control(shared: Shared) -> Result<(), &'static str> {
    let Ok(mutex) = Arc::try_unwrap(shared) else {
        return Err("Control owner still shared at shutdown.");
    };
    let mut driver = mutex.into_inner();
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
    control: Option<Shared>,
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
