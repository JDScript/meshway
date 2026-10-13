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

use super::{Selection, intent::Desired, network, status::Report};
use crate::settings::Settings;
use cycle::{Online, online};
use failure::failure;
use meshway_control::{Control, Error};
use meshway_model::{InstanceId, NetmapRef};
use meshway_protocol::ipc::{Phase, Problem};
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
/// Control retry backoff cap (seconds), the same as the sync loop's: after a
/// Server outage the Node is back within one backoff period.
const BACKOFF_MAX: u64 = 15;
/// A sync that has not succeeded for this long is reported as reconnecting:
/// above one Server hold plus the request budget's margin.
pub(super) const SYNC_STALE: Duration = Duration::from_secs(100);

/// The Control owner's private directory inside the daemon's state directory.
pub(super) const IDENTITY_DIRECTORY: &str = "identity";

type Shared = Arc<Mutex<Control>>;

/// The `sync` long poll as seen by status: whether it runs, and when it last
/// returned a verified netmap.
#[derive(Clone, Debug, Default)]
pub(crate) struct SyncStatus {
    pub running: bool,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_sync: Option<std::time::Instant>,
    /// Class of the last failure while the long poll retries.
    pub error: Option<meshway_protocol::ipc::SyncError>,
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
        logout: None,
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
        clock_floor_ms: control.clock_floor_ms(),
    });
}

/// End of one up session.
enum Ended {
    /// The operator intent changed (or shutdown).
    Changed,
    /// A failure that only a new operator revision can retry. `withdraw`
    /// ends the data plane too (decoded withdrawal or unknown storage);
    /// otherwise the held netmap keeps running to its lease end.
    Blocked { problem: Problem, withdraw: bool },
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
    local_pings: watch::Sender<Option<network::Local>>,
    server_probe: watch::Sender<Option<meshway_control::ServerProbe>>,
    sync_status: watch::Sender<SyncStatus>,
    mut selections: tokio::sync::mpsc::Receiver<Selection>,
) -> Result<(), &'static str> {
    let mut control: Option<Shared> = None;
    let mut network: Option<network::Handle> = None;
    let mut plan = desired.borrow_and_update().clone();
    let mut backoff = 1u64;
    let mut reset = plan.retry_enrollment;
    let mut reenroll = plan.reenroll;
    let mut shutdown_failure = None;
    let mut left = false;
    loop {
        if plan.shutdown || desired.has_changed().is_err() {
            break;
        }
        if control.is_none() {
            if settings.network_mode == meshway_protocol::ipc::NetworkMode::Tun && network.is_none()
            {
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
                        local_pings.send_replace(Some(handle.local()));
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
                        backoff = (backoff * 2).min(BACKOFF_MAX);
                        wait_change(&mut desired, &plan, delay).await;
                        plan = desired.borrow_and_update().clone();
                        reset = plan.retry_enrollment;
                        reenroll = plan.reenroll;
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
                        &settings.data_dir.join(IDENTITY_DIRECTORY),
                        &settings.server,
                        builder,
                    )
                })
                .await
            };
            match opened {
                Ok(Ok(value)) => {
                    server_probe.send_replace(Some(value.server_probe()));
                    control = Some(Arc::new(Mutex::new(value)));
                }
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
                    reenroll = plan.reenroll;
                    continue;
                }
            }
            let latest = desired.borrow_and_update().clone();
            if latest.revision != plan.revision || latest.shutdown {
                plan = latest;
                reset = plan.retry_enrollment;
                reenroll = plan.reenroll;
                continue;
            }
        }
        let shared = control.clone().expect("control opened above");
        let leaving = shared.lock().await.leaving();
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
                        Error::Pending => Some(meshway_protocol::ipc::ErrorCode::Pending),
                        Error::Lifecycle => Some(meshway_protocol::ipc::ErrorCode::NotReady),
                        _ => None,
                    };
                    let problem = failure::at("rotate_keys", error, true).problem;
                    local.unwrap_or(match problem {
                        Problem::Network => meshway_protocol::ipc::ErrorCode::Network,
                        Problem::ServerBusy => meshway_protocol::ipc::ErrorCode::ServerBusy,
                        Problem::Credentials => meshway_protocol::ipc::ErrorCode::Credentials,
                        Problem::Storage => meshway_protocol::ipc::ErrorCode::Storage,
                        Problem::Clock | Problem::Apply => {
                            meshway_protocol::ipc::ErrorCode::NotReady
                        }
                        _ => meshway_protocol::ipc::ErrorCode::Contract,
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
        } else if leaving.is_some() && plan.logout.is_none() && plan.enrollment_key.is_some() {
            // An explicit new enrollment replaces the pending leave.
            let mut driver = shared.lock().await;
            stop(&mut driver, false).await;
            match driver.abandon_leave().await {
                Ok(()) => {
                    tracing::warn!("A new enrollment key replaced the pending leave");
                    if let Some(network) = &mut network {
                        let _ = network.stop(true).await;
                    }
                    left = true;
                }
                Err(error) => {
                    publish(
                        &reported,
                        &plan,
                        Some(&driver),
                        Phase::Blocked,
                        None,
                        Some(failure(error, true).problem),
                        None,
                    );
                    drop(driver);
                    wait_change(&mut desired, &plan, None).await;
                }
            }
        } else if let Some(id) = plan.logout.or(leaving) {
            let mut driver = shared.lock().await;
            // No session work runs meanwhile, and the data plane stops: the
            // operator asked to leave. The request needs a session only.
            stop(&mut driver, false).await;
            if let Some(network) = &mut network {
                let _ = network.stop(false).await;
            }
            publish(
                &reported,
                &plan,
                Some(&driver),
                Phase::Leaving,
                None,
                None,
                None,
            );
            let attempt = driver.leave(id).await;
            let pending = driver.leaving().is_some();
            let mut problem = None;
            let result = match attempt {
                Ok(left) => {
                    match left {
                        meshway_control::Left::Confirmed(left) => tracing::info!(
                            network = %left.network_id, node = %left.node_id,
                            revision = left.revision.get(), "Node left its network"),
                        meshway_control::Left::IdentityRejected => {
                            tracing::info!("Node left its network: the Server rejects its identity")
                        }
                    }
                    Ok(())
                }
                // The intent stays and is retried: no Server answer yet.
                Err(error) if pending => {
                    problem = Some(failure::at("leave", error, true).problem);
                    Err(meshway_protocol::ipc::ErrorCode::Leaving)
                }
                Err(error) => Err(leave_error(error)),
            };
            if result.is_ok() {
                // The Server withdrew everything; remove every local resource.
                if let Some(network) = &mut network {
                    let _ = network.stop(true).await;
                }
                left = true;
            }
            let retry = pending.then(|| Duration::from_secs(backoff));
            publish(
                &reported,
                &plan,
                Some(&driver),
                match &result {
                    Ok(()) => Phase::Down,
                    Err(_) if pending => Phase::Leaving,
                    Err(_) => Phase::Starting,
                },
                None,
                problem,
                retry,
            );
            drop(driver);
            // The operator's request, or a retried leave that completed,
            // reports its outcome; the coordinator settles the intent.
            if plan.logout.is_some() || result.is_ok() {
                reported.send_modify(|report| {
                    report.logout = Some(super::status::RotationOutcome { id, result })
                });
            }
            match retry {
                Some(delay) => {
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    wait_change(&mut desired, &plan, Some(delay)).await;
                    // Keep the backoff across the next attempt.
                    plan = desired.borrow_and_update().clone();
                    reset = plan.retry_enrollment;
                    reenroll = plan.reenroll;
                    continue;
                }
                None if plan.logout.is_some() => {
                    let _ = desired
                        .wait_for(|next| next.shutdown || next.logout != Some(id))
                        .await;
                }
                None if result.is_ok() => {
                    wait_change(&mut desired, &plan, None).await;
                }
                // A retried leave was refused: the intent is gone, the held
                // identity carries on.
                None => {}
            }
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
                &mut reenroll,
                &mut selections,
            )
            .await;
            sync_status.send_modify(|status| {
                status.running = false;
                status.error = None;
            });
            match ended {
                Ended::Changed => {}
                Ended::Blocked { problem, withdraw } => {
                    let mut driver = shared.lock().await;
                    if withdraw {
                        stop(&mut driver, false).await;
                        if let Some(network) = &mut network {
                            let _ = network.stop(false).await;
                        }
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
        // A Node that left reopens its now empty identity record.
        if reload || std::mem::take(&mut left) {
            drop(shared);
            server_probe.send_replace(None);
            if let Some(driver) = control.take()
                && let Err(error) = finish_control(driver).await
            {
                shutdown_failure = Some(error);
                break;
            }
        }
        plan = desired.borrow_and_update().clone();
        reset = plan.retry_enrollment;
        reenroll = plan.reenroll;
        backoff = 1;
    }
    let keep_sink = desired.borrow().up;
    local_pings.send_replace(None);
    server_probe.send_replace(None);
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

/// The local result of a failed self-deletion; nothing local changed.
fn leave_error(error: Error) -> meshway_protocol::ipc::ErrorCode {
    operation_error("leave", error)
}

/// The local result of a failed routing profile selection: the Server's
/// refusal of the profile or of the held version, or the request's failure.
fn selection_error(error: Error) -> meshway_protocol::ipc::ErrorCode {
    use meshway_configuration::selection::SelectionUnavailable;
    use meshway_protocol::{Problem, ipc::ErrorCode};
    if let Error::Server(meshway_control_client::ClientError::Http {
        problem: Some(body),
        ..
    }) = &error
    {
        match &body.problem {
            Problem::RoutingProfileUnavailable { params, .. } => {
                return match params.reason {
                    SelectionUnavailable::NotAllowed => ErrorCode::ProfileNotAllowed,
                    SelectionUnavailable::Locked => ErrorCode::ProfileLocked,
                };
            }
            Problem::VersionConflict { .. } => return ErrorCode::SelectionConflict,
            _ => {}
        }
    }
    if matches!(error, Error::Expired) {
        return ErrorCode::NotReady;
    }
    operation_error("select_profile", error)
}

/// Send one operator selection unless the operator stopped waiting.
async fn serve_selection(shared: &Shared, selection: Selection) {
    if selection.reply.is_closed() || Instant::now() >= selection.deadline {
        return;
    }
    let profile = selection.profile.clone();
    let result = tokio::time::timeout_at(
        selection.deadline,
        meshway_control::shared::select_profile(shared, selection.profile),
    )
    .await
    .unwrap_or(Err(Error::Pending))
    .map_err(selection_error);
    match &result {
        Ok(state) => tracing::info!(
            profile,
            applied = state.applied_profile,
            version = state.version.map(|version| version.get()),
            "Routing profile selected"
        ),
        Err(code) => tracing::warn!(profile, error = ?code, "Routing profile selection failed"),
    }
    let _ = selection.reply.send(result);
}

fn operation_error(operation: &'static str, error: Error) -> meshway_protocol::ipc::ErrorCode {
    use meshway_protocol::ipc::ErrorCode;
    let rejected = matches!(
        &error,
        Error::Server(meshway_control_client::ClientError::Http {
            status: 403 | 409 | 422,
            problem: Some(body),
        }) if !matches!(body.problem, meshway_protocol::Problem::NodeIdentityRejected { .. })
    );
    let local = match &error {
        Error::Pending => Some(ErrorCode::Pending),
        Error::Lifecycle => Some(ErrorCode::NotReady),
        _ => None,
    };
    let problem = failure::at(operation, error, false).problem;
    if rejected {
        return ErrorCode::Rejected;
    }
    local.unwrap_or(match problem {
        Problem::Network => ErrorCode::Network,
        Problem::ServerBusy => ErrorCode::ServerBusy,
        Problem::Credentials => ErrorCode::Credentials,
        Problem::Storage => ErrorCode::Storage,
        Problem::Clock | Problem::Apply => ErrorCode::NotReady,
        _ => ErrorCode::Contract,
    })
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
    selections: &mut tokio::sync::mpsc::Receiver<Selection>,
) -> Wake {
    let deadline = Instant::now() + delay;
    loop {
        tokio::select! { biased;
            _ = changed(desired, plan) => return Wake::Changed,
            _ = tokio::time::sleep_until(deadline) => return Wake::Due,
            Some(selection) = selections.recv() => serve_selection(shared, selection).await,
            work = async { match network.as_deref_mut() { Some(network) => network.work().await, None => std::future::pending().await } } => match work {
                network::Work::Request(request) => {
                    if let Err(error) = network::request(shared, request).await {
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
    reenroll: &mut bool,
    selections: &mut tokio::sync::mpsc::Receiver<Selection>,
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
        // A verified held netmap is authority again without Server contact,
        // and its saved last known peer facts are dial addresses again.
        offer(&driver, network.as_deref());
        if let Some(network) = network.as_deref()
            && let Some(known) = driver.known_peers()
        {
            use meshway_control::PeerFactsSink as _;
            network.peer_sink().publish(known);
        }
    }
    let mut backoff = 1u64;
    loop {
        // Enrollment and authentication.
        let result = {
            let mut driver = shared.lock().await;
            let result = tokio::select! { biased;
                _ = changed(desired, plan) => return Ended::Changed,
                result = online(&mut driver, settings, reset, reenroll, plan.enrollment_key.as_ref()) => result,
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
                Ok(Online::Ready) => {
                    publish(
                        reported,
                        plan,
                        Some(&driver),
                        Phase::ControlReady,
                        None,
                        None,
                        None,
                    );
                    // The session's sync forwards verified netmaps straight to
                    // Engine: its packet keys and instance (first known after
                    // enrollment/authentication) must already be there.
                    offer(&driver, network.as_deref());
                }
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
                    selections,
                )
                .await
                {
                    Ok(()) => continue,
                    Err(Some(ended)) => return ended,
                    Err(None) => {
                        let delay = Duration::from_secs(backoff);
                        backoff = (backoff * 2).min(BACKOFF_MAX);
                        delay
                    }
                }
            }
            Ok(Online::Awaiting { .. }) => Duration::from_secs(2),
            Ok(Online::Restart) => continue,
            Err(error) if !error.retry => {
                return Ended::Blocked {
                    problem: error.problem,
                    withdraw: error.withdraw,
                };
            }
            Err(error) => {
                let delay = error.retry_after.unwrap_or(Duration::from_secs(backoff));
                backoff = (backoff * 2).min(BACKOFF_MAX);
                delay
            }
        };
        match idle(
            shared,
            network.as_deref_mut(),
            desired,
            plan,
            delay,
            selections,
        )
        .await
        {
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
    selections: &mut tokio::sync::mpsc::Receiver<Selection>,
) -> Result<(), Option<Ended>> {
    let stop = CancellationToken::new();
    // Selections run beside the maintenance work and end with the session.
    let mut selecting = tokio::task::JoinSet::new();
    let (netmaps, mut netmap_rx) = watch::channel::<Option<meshway_engine::Netmap>>(None);
    netmap_rx.borrow_and_update();
    let mut sink = match network.as_deref() {
        Some(network) => network.peer_sink(),
        None => watch::channel(None).0,
    };
    let (errors, mut error_rx) = watch::channel(None);
    let mut errors_open = true;
    let mut sync = {
        let shared = shared.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            meshway_control::sync_loop(&shared, &mut sink, &netmaps, &errors, &stop).await
        })
    };
    // The verified lease reaches Engine without the Control lock or this
    // session's maintenance select: no network wait lies on that path.
    let forward = network.as_deref().map(|network| {
        let leases = network.lease_sink();
        let mut netmaps = netmap_rx.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! { biased;
                    _ = stop.cancelled() => return,
                    changed = netmaps.changed() => if changed.is_err() { return },
                }
                let netmap = netmaps.borrow_and_update().clone();
                leases.offer_netmap(netmap);
            }
        })
    });
    sync_status.send_modify(|status| status.running = true);
    let mut work = Maintenance::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! { biased;
            _ = changed(desired, plan) => break Err(Some(Ended::Changed)),
            _ = selecting.join_next(), if !selecting.is_empty() => {},
            Some(selection) = selections.recv(), if selecting.len() < 4 => {
                let shared = shared.clone();
                selecting.spawn(async move { serve_selection(&shared, selection).await });
            },
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
                    break Err(Some(Ended::Blocked {
                        problem: failure.problem,
                        withdraw: failure.withdraw,
                    }));
                }
                publish(reported, plan, Some(&driver), Phase::Retrying, None, Some(failure.problem), None);
                break Err(None);
            },
            changed = error_rx.changed(), if errors_open => {
                if changed.is_err() {
                    errors_open = false;
                    continue;
                }
                let error = *error_rx.borrow_and_update();
                if let Some(error) = error {
                    tracing::warn!(?error, "Node sync failed; retrying");
                }
                sync_status.send_modify(|status| status.error = error);
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
                    if let Err(error) = network::request(shared, request).await {
                        tracing::debug!(%error, "Network control intent failed");
                    }
                },
                network::Work::Report => {
                    if let Err(ended) = maintain(shared, network.as_deref_mut(), settings, &mut work, plan, reported, true).await {
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
                if let Err(ended) = maintain(shared, network.as_deref_mut(), settings, &mut work, plan, reported, false).await {
                    break Err(Some(ended));
                }
            },
        }
    };
    stop.cancel();
    selecting.shutdown().await;
    if !sync.is_finished() {
        let _ = sync.await;
    }
    if let Some(forward) = forward {
        let _ = forward.await;
    }
    result
}

/// Periodic instance, State, receipt and observation-task work. Transient
/// failures are retried after a backoff and never stop the data plane;
/// identity and binding rejections block. The Control lock is taken per
/// operation, so a sync reply waits for at most one operation, never for the
/// whole sequence.
#[allow(clippy::too_many_arguments)]
async fn maintain(
    shared: &Shared,
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
    let result = maintain_once(shared, network.as_deref_mut(), settings, work, changed).await;
    let control = &*shared.lock().await;
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
        Err((operation, error @ (Error::StateScopeChanged | Error::StaleReport))) => {
            tracing::debug!(operation, %error, "State observation not current; resampling");
            work.retry_at = Some(now + Duration::from_secs(1));
            Ok(())
        }
        Err((operation, error)) => {
            let failure = failure::at(operation, error, operation == "authenticate");
            offer(control, network.as_deref());
            if !failure.retry {
                return Err(Ended::Blocked {
                    problem: failure.problem,
                    withdraw: failure.withdraw,
                });
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
    shared: &Shared,
    network: Option<&mut network::Handle>,
    settings: &Settings,
    work: &mut Maintenance,
    changed: bool,
) -> Result<(), (&'static str, Error)> {
    use meshway_control::shared as control;
    let now = Instant::now();
    control::authenticate(shared)
        .await
        .map_err(|error| ("authenticate", error))?;
    let mut fresh = false;
    if shared.lock().await.status().instance.is_none() {
        control::acquire_instance(
            shared,
            if network.is_some() {
                network::capabilities()
            } else {
                Default::default()
            },
        )
        .await
        .map_err(|error| ("acquire_instance", error))?;
        work.renewed = Some(now);
        work.receipted = None;
        fresh = true;
        // State/receipt binding of the new instance; nothing reinstalls.
        offer(&*shared.lock().await, network.as_deref());
    } else if work.renewed.is_none_or(|at| now >= at + RENEW) {
        control::renew_instance(shared)
            .await
            .map_err(|error| ("renew_instance", error))?;
        work.renewed = Some(now);
    }
    if let Some(network) = network {
        receipt(shared, network, work).await?;
        let routine = fresh || work.reported.is_none_or(|at| now >= at + REPORT);
        if (routine || changed)
            && network
                .report_state(shared, settings, routine)
                .await
                .map_err(|error| ("report_runtime_state", error))?
        {
            work.reported = Some(now);
        }
        let netmap = shared.lock().await.netmap_lease().map(|lease| lease.netmap);
        if work.tasks.is_none_or(|at| now >= at + TASKS)
            && let Some(netmap) = netmap
        {
            work.tasks = Some(now);
            let tasks = control::get_observation_tasks(shared, netmap)
                .await
                .map_err(|error| ("get_observation_tasks", error))?;
            network.observation_tasks(netmap, tasks.items);
        }
    } else if fresh || work.reported.is_none_or(|at| now >= at + REPORT) {
        let state = shared
            .lock()
            .await
            .control_state(settings.system.clone())
            .map_err(|error| ("report_control_state", error))?;
        match control::report_runtime_state(shared, state, chrono::Utc::now()).await {
            Ok(_) => work.reported = Some(now),
            Err(Error::Pending) => {
                control::retry_state_report(shared)
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
    shared: &Shared,
    network: &mut network::Handle,
    work: &mut Maintenance,
) -> Result<(), (&'static str, Error)> {
    let report = network.report.borrow().clone();
    let control = shared.lock().await;
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
    drop(control);
    let ticket = meshway_control::shared::begin_application(shared, netmap)
        .await
        .map_err(|error| ("begin_application", error))?;
    meshway_control::shared::finish_application(shared, &ticket, completion, report.observed_at)
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
