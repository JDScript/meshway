//! One enrollment/authentication/configuration/report cycle through meshway-control.
use super::failure::{self, Failure};
use crate::runtime::network;
use crate::settings::Settings;
use meshway_control::{Control, Error};
use meshway_ipc::{Phase, Problem};
use meshway_protocol::enrollment::EnrollmentPhase;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);

/// Timings describe real owner work, including a dropped lifecycle future. No
/// request bodies, credentials, or synthetic observations enter these events.
async fn stage<T, E>(
    name: &'static str,
    operation: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    struct Span {
        name: &'static str,
        started: std::time::Instant,
        outcome: &'static str,
    }
    impl Drop for Span {
        fn drop(&mut self) {
            tracing::debug!(
                stage = self.name,
                elapsed_ms = self.started.elapsed().as_millis() as u64,
                outcome = self.outcome,
                "Control stage completed"
            );
        }
    }
    let mut span = Span {
        name,
        started: std::time::Instant::now(),
        outcome: "cancelled",
    };
    tracing::debug!(stage = name, "Control stage started");
    let result = operation.await;
    span.outcome = if result.is_ok() { "ok" } else { "error" };
    result
}

pub(super) struct Cycle {
    pub phase: Phase,
    pub url: Option<String>,
    pub delay: Duration,
    pub renewed: bool,
    pub reported: bool,
}
pub(super) async fn cycle(
    control: &mut Control,
    settings: &Settings,
    renew: bool,
    report: bool,
    reset: &mut bool,
    mut network: Option<&mut network::Handle>,
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
                .map_err(|e| failure::at("begin_enrollment", e, true))?;
            let view = control
                .poll_enrollment()
                .await
                .map_err(|e| failure::at("poll_enrollment", e, true))?;
            match view.phase {
                EnrollmentPhase::Pending => {
                    *reset = false;
                    return Ok(Cycle {
                        phase: Phase::AwaitingApproval,
                        url: created.authorization_url,
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
                            .map_err(|e| failure::at("cancel_enrollment", e, true))?;
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
                        reset: false,
                        retry_after: None,
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
                        reset: false,
                        retry_after: None,
                    });
                }
            }
            return Err(failure::at("redeem_enrollment", error, true));
        }
    }
    if control.rotation_pending() {
        control
            .rotate_keys()
            .await
            .map_err(|e| failure::at("rotate_keys", e, true))?;
    }
    stage("authenticate", control.authenticate())
        .await
        .map_err(|e| failure::at("authenticate", e, true))?;
    let mut renewed = false;
    if control.status().instance.is_none() {
        control
            .acquire_instance(if network.is_some() {
                network::capabilities()
            } else {
                Default::default()
            })
            .await
            .map_err(|e| failure::at("acquire_instance", e, false))?;
        renewed = true;
    } else if renew && network.is_none() {
        control
            .renew_instance()
            .await
            .map_err(|e| failure::at("renew_instance", e, false))?;
        renewed = true;
    }
    let before = control.status().cached;
    if network.is_some() {
        stage("refresh_configuration", control.refresh_configuration())
            .await
            .map_err(|e| failure::at("refresh_configuration", e, false))?;
        // Refresh grants every TUN poll; reporting/renewal scheduling still
        // records the normal renewal cadence rather than manufacturing samples.
        renewed |= renew;
    } else {
        stage("sync_configuration", control.sync_configuration())
            .await
            .map_err(|e| failure::at("sync_configuration", e, false))?;
    }
    if control
        .status()
        .instance
        .as_ref()
        .is_some_and(|i| i.target.is_some())
    {
        stage("acknowledge_saved", control.acknowledge_saved())
            .await
            .map_err(|e| failure::at("acknowledge_saved", e, false))?;
    }
    if let Some(network) = &mut network {
        // The atomic refresh already delivered the granted target. A remaining
        // unavailable grant is a real installation barrier, not a reason to
        // observe more publications in a lease/query retry dance.
        if control
            .status()
            .instance
            .as_ref()
            .is_none_or(|instance| instance.target.is_none())
            || (enabled_current_target(control) && control.execution_snapshot().is_err())
        {
            return Err(Failure {
                problem: Problem::Apply,
                retry: true,
                reset: false,
                retry_after: None,
            });
        }
        // A retained historical completion may wait for HTTP retry, but cannot
        // prevent offering a newly granted, fsynced target to the network owner.
        stage("publish", network.publish(control))
            .await
            .map_err(|error| failure::at("publish", error, false))?;
        // The bounded plan watch waits for this exact installed snapshot. Fetch
        // during application rather than losing a full poll after each rebind.
        network.observation_tasks(control).await;
        stage("finish", network.finish(control))
            .await
            .map_err(|error| failure::at("finish_application", error, false))?;
        stage("publish_after_finish", network.publish(control))
            .await
            .map_err(|error| failure::at("publish_after_finish", error, false))?;
    }
    let routine = report || renewed || before != control.status().cached;
    let reported = if let Some(network) = &mut network {
        stage(
            "report_state",
            network.report_state(control, settings, routine),
        )
        .await
        .map_err(|error| failure::at("report_runtime_state", error, false))?
    } else if routine {
        match control.report_control_state(settings.system.clone()).await {
            Ok(_) => true,
            Err(Error::Pending) => {
                control
                    .retry_state_report()
                    .await
                    .map_err(|e| failure::at("retry_state_report", e, false))?;
                false
            }
            Err(error) => return Err(failure::at("report_control_state", error, false)),
        }
    } else {
        false
    };
    Ok(Cycle {
        phase: Phase::ControlReady,
        url: None,
        delay: if network
            .as_ref()
            .is_some_and(|network| network.preparing_dns())
        {
            Duration::from_millis(100)
        } else {
            POLL
        },
        renewed,
        reported,
    })
}

fn enabled_current_target(control: &Control) -> bool {
    let status = control.status();
    control
        .cached_target()
        .is_some_and(|target| target.configuration.enabled)
        && status.cached.is_some()
        && status
            .instance
            .as_ref()
            .is_some_and(|instance| instance.target == status.cached)
}

#[cfg(test)]
mod tests {
    use super::failure::failure;
    use super::*;

    #[test]
    fn state_scope_rejection_requests_prompt_refresh_without_credential_failure() {
        for authenticating in [false, true] {
            let retry = failure(Error::StateScopeChanged, authenticating);
            assert_eq!(retry.problem, Problem::Apply);
            assert!(retry.retry);
            assert_eq!(retry.retry_after, Some(Duration::from_secs(1)));
        }
        let invalid = failure(Error::Contract, false);
        assert_eq!(invalid.problem, Problem::Contract);
        assert!(!invalid.retry);
        assert_eq!(invalid.retry_after, None);
    }
}
