//! Enrollment and authentication through meshway-control: everything that
//! must happen before the `sync` long poll can run.
use super::failure::{self, Failure};
use crate::settings::Settings;
use meshway_control::Control;
use meshway_ipc::Problem;
use meshway_protocol::enrollment::EnrollmentPhase;

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

pub(super) enum Online {
    /// Waiting for an administrator to approve enrollment.
    Awaiting { url: Option<String> },
    /// A terminal enrollment was discarded; start over at once.
    Restart,
    /// Enrolled and authenticated: `sync` may run.
    Ready,
}

pub(super) async fn online(
    control: &mut Control,
    settings: &Settings,
    reset: &mut bool,
    enrollment_key: Option<&meshway_ipc::EnrollmentKey>,
) -> Result<Online, Failure> {
    if control.status().binding.is_none()
        && let Some(key) = enrollment_key
    {
        let hostname = control
            .pending_hostname()
            .unwrap_or(&settings.hostname)
            .to_owned();
        control
            .enroll_with_key(
                &hostname,
                meshway_protocol::WireSecret::new(key.expose().to_owned()),
            )
            .await
            .map_err(|e| failure::at("enroll_with_key", e, true))?;
    }
    if control.key_enrollment_pending() {
        control
            .resume_key_enrollment()
            .await
            .map_err(|e| failure::at("enroll_with_key", e, true))?;
    }
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
                    return Ok(Online::Awaiting {
                        url: created.authorization_url,
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
                        return Ok(Online::Restart);
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
    Ok(Online::Ready)
}
