use crate::{
    settings::Settings,
    status::{Desired, Report},
};
use meshway_control::{Control, Error};
use meshway_control_client::ClientError;
use meshway_ipc::{Phase, Problem};
use meshway_protocol::enrollment::EnrollmentPhase;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

const POLL: Duration = Duration::from_secs(5);
const RENEW: Duration = Duration::from_secs(20);
const REPORT: Duration = Duration::from_secs(20);
const RELEASE: Duration = Duration::from_secs(2);

struct Cycle {
    phase: Phase,
    url: Option<String>,
    delay: Duration,
    renewed: bool,
    reported: bool,
}
struct Failure {
    problem: Problem,
    retry: bool,
}

fn failure(error: Error, authenticating: bool) -> Failure {
    let (problem, retry) = match error {
        Error::Server(ClientError::Network { .. }) => (Problem::Network, true),
        Error::Server(ClientError::Http {
            status: 429 | 503 | 502 | 504,
            ..
        }) => (Problem::ServerBusy, true),
        Error::Server(ClientError::Http {
            status: 401 | 403 | 404 | 409,
            ..
        }) => (Problem::Credentials, !authenticating),
        Error::Expired => (Problem::Clock, true),
        Error::Storage(_) | Error::Record | Error::ReloadRequired => (Problem::Storage, false),
        Error::Endpoint => (Problem::Configuration, false),
        _ => (Problem::Contract, false),
    };
    Failure { problem, retry }
}

async fn cycle(
    control: &mut Control,
    settings: &Settings,
    renew: bool,
    report: bool,
    reset: &mut bool,
) -> Result<Cycle, Failure> {
    if control.status().binding.is_none() {
        if !control.can_redeem_enrollment() {
            let hostname = control
                .pending_hostname()
                .unwrap_or(&settings.hostname)
                .to_owned();
            let created = control
                .begin_enrollment(&hostname)
                .await
                .map_err(|e| failure(e, true))?;
            let view = control
                .poll_enrollment()
                .await
                .map_err(|e| failure(e, true))?;
            match view.phase {
                EnrollmentPhase::Pending => {
                    *reset = false;
                    return Ok(Cycle {
                        phase: Phase::AwaitingApproval,
                        url: Some(format!(
                            "{}authorize-device/{}",
                            settings.console_origin, created.id
                        )),
                        delay: Duration::from_secs(2),
                        renewed: false,
                        reported: false,
                    });
                }
                EnrollmentPhase::Expired
                | EnrollmentPhase::Withdrawn
                | EnrollmentPhase::Cancelled => {
                    if *reset {
                        control
                            .cancel_enrollment()
                            .await
                            .map_err(|e| failure(e, true))?;
                        *reset = false;
                        return Ok(Cycle {
                            phase: Phase::Starting,
                            url: None,
                            delay: Duration::ZERO,
                            renewed: false,
                            reported: false,
                        });
                    }
                    let problem = match view.phase {
                        EnrollmentPhase::Expired => Problem::EnrollmentExpired,
                        EnrollmentPhase::Withdrawn => Problem::EnrollmentWithdrawn,
                        _ => Problem::EnrollmentCancelled,
                    };
                    return Err(Failure {
                        problem,
                        retry: false,
                    });
                }
                EnrollmentPhase::Approved | EnrollmentPhase::Consumed => {}
            }
        }
        if let Err(error) = control.redeem_enrollment().await {
            // A terminal request may have changed while its redemption was in
            // flight. Keep the durable proof/code and expose the real phase.
            if let Ok(view) = control.poll_enrollment().await {
                let problem = match view.phase {
                    EnrollmentPhase::Expired => Some(Problem::EnrollmentExpired),
                    EnrollmentPhase::Withdrawn => Some(Problem::EnrollmentWithdrawn),
                    EnrollmentPhase::Cancelled => Some(Problem::EnrollmentCancelled),
                    _ => None,
                };
                if let Some(problem) = problem {
                    return Err(Failure {
                        problem,
                        retry: false,
                    });
                }
            }
            return Err(failure(error, true));
        }
    }
    control.authenticate().await.map_err(|e| failure(e, true))?;
    let mut renewed = false;
    if control.status().instance.is_none() {
        control
            .acquire_instance(Default::default())
            .await
            .map_err(|e| failure(e, false))?;
        renewed = true;
    } else if renew {
        control
            .renew_instance()
            .await
            .map_err(|e| failure(e, false))?;
        renewed = true;
    }
    let before = control.status().cached;
    control
        .sync_configuration()
        .await
        .map_err(|e| failure(e, false))?;
    if control
        .status()
        .instance
        .as_ref()
        .is_some_and(|i| i.target.is_some() && i.execution_expires_at_ms.is_some())
    {
        control
            .acknowledge_saved()
            .await
            .map_err(|e| failure(e, false))?;
    }
    let reported = report || renewed || before != control.status().cached;
    if reported {
        match control.report_control_state(settings.system.clone()).await {
            Ok(_) => {}
            Err(Error::Pending) => {
                control
                    .retry_state_report()
                    .await
                    .map_err(|e| failure(e, false))?;
            }
            Err(error) => return Err(failure(error, false)),
        }
    }
    Ok(Cycle {
        phase: Phase::ControlReady,
        url: None,
        delay: POLL,
        renewed,
        reported,
    })
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
    let sleep = async {
        match delay {
            Some(delay) => tokio::time::sleep(delay).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(sleep);
    loop {
        tokio::select! { biased;
            result = receiver.changed() => {
                if result.is_err() { return true; }
                let next = receiver.borrow_and_update();
                if next.shutdown || next.revision != plan.revision { return true; }
            },
            _ = &mut sleep => return false,
        }
    }
}

pub async fn run(
    settings: Arc<Settings>,
    mut desired: watch::Receiver<Desired>,
    reported: watch::Sender<Report>,
) {
    let mut control: Option<Control> = None;
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
            let settings = settings.clone();
            let opened = tokio::task::spawn_blocking(move || {
                Control::open(
                    &settings.data_dir.join("identity"),
                    &settings.server,
                    reqwest::Client::builder().no_proxy(),
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
            publish(
                &reported,
                &plan,
                Some(driver),
                Phase::Down,
                None,
                None,
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
            let mut operation = Box::pin(cycle(
                driver,
                &settings,
                last_renew.is_none_or(|t| t.elapsed() >= RENEW),
                last_report.is_none_or(|t| t.elapsed() >= REPORT),
                &mut reset,
            ));
            let outcome = loop {
                tokio::select! { biased;
                    result = desired.changed() => {
                        if result.is_err() { break None; }
                        let next = desired.borrow_and_update();
                        if next.shutdown || next.revision != plan.revision { break None; }
                    },
                    result = &mut operation => break Some(result),
                }
            };
            drop(operation);
            match outcome {
                Some(Ok(result)) => {
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
                    if !wait_change(&mut desired, &plan, Some(result.delay)).await {
                        continue;
                    }
                }
                Some(Err(error)) => {
                    let delay = error.retry.then_some(Duration::from_secs(backoff));
                    if !error.retry {
                        stop(driver, false).await;
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
                    if !wait_change(&mut desired, &plan, delay).await {
                        continue;
                    }
                }
                None => {}
            }
        }
        stop(control.as_mut().expect("control exists"), false).await;
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
}
