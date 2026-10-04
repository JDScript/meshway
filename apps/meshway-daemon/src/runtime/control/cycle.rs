//! One enrollment/authentication/configuration/report cycle through meshway-control.
use crate::settings::Settings;
use meshway_control::{Control, Error};
use meshway_control_client::ClientError;
use meshway_ipc::{Phase, Problem};
use meshway_protocol::enrollment::EnrollmentPhase;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);

pub(super) struct Cycle {
    pub phase: Phase,
    pub url: Option<String>,
    pub delay: Duration,
    pub renewed: bool,
    pub reported: bool,
}
pub(super) struct Failure {
    pub problem: Problem,
    pub retry: bool,
}

pub(super) fn failure(error: Error, authenticating: bool) -> Failure {
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

pub(super) async fn cycle(
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
