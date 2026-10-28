//! Ending the packet plane: shutdown, failure and lease withdrawal, and the
//! reinstall that ends a failure withdrawal.
use super::super::{plane::Plane, reporting::Publisher};
use meshway_engine::Engine;
use meshway_protocol::execution::ApplyFailure;
use meshway_protocol::ipc::{DataPlane, Problem};

pub(super) async fn shutdown(
    mut engine: Engine<Plane>,
    dns_queries: &mut super::super::dns::mesh::Queries,
    diagnostics: &mut super::super::diagnostics::Diagnostics,
    keep_sink: bool,
) -> Result<(), Problem> {
    dns_queries.stop().await;
    diagnostics.stop().await;
    let closing = engine.reconfig(None).await.map_err(|_| Problem::Underlay);
    let mut plane = engine.into_components();
    let dns_result = plane
        .dns
        .shutdown(&mut plane.host, keep_sink)
        .await
        .map_err(|_| Problem::Dns);
    let host_result = plane
        .host
        .shutdown(keep_sink)
        .await
        .map_err(|_| Problem::Underlay);
    dns_result.and(closing).and(host_result)
}

/// Control withdrew the lease or the Host lost its physical owner: apply
/// empty, which terminates every flow, circuit and exit projection and keeps
/// the negative capture and DNS boundary.
pub(super) async fn withdraw(engine: &mut Engine<Plane>, reports: &Publisher, host_failed: bool) {
    tracing::info!(
        host_failed,
        "Netmap withdrawn; terminating the packet plane"
    );
    let cleanup = engine.reconfig(None).await;
    finish_withdrawal(engine, reports, host_failed || cleanup.is_err());
}

pub(super) fn finish_withdrawal(engine: &mut Engine<Plane>, reports: &Publisher, failed: bool) {
    let failed = failed || engine.components().failure.is_some();
    engine.components_mut().failure = None;
    let mut report = reports.borrow().clone();
    report.plane = if failed {
        DataPlane::Failed
    } else {
        DataPlane::Suspended
    };
    report.problem = failed.then_some(Problem::Underlay);
    // Why kernel ownership ended, while it stays ended: never an empty
    // detail over a known reason.
    report.problem_detail = engine
        .components()
        .host
        .ownership_ended()
        .filter(|_| failed)
        .map(|end| end.to_string());
    report.completion = None;
    report.content = None;
    report.expires_at = None;
    report.facts = None;
    report.listeners = None;
    report.exit_flows = None;
    if let Some(capability) = &mut report.exit_capability {
        capability.set_installed(false);
    }
    reports.send_replace(report);
}

/// The worker's own retry ended without an installation (nothing to
/// install): Status no longer shows its reinstall (`retryAt`) or refusal.
pub(super) fn retry_ended(reports: &Publisher, retry: &meshway_model::recovery::Recovery) {
    let mut report = reports.borrow().clone();
    report.apply = retry.health().clone();
    report.observed_at = chrono::Utc::now();
    reports.send_replace(report);
}

/// Publish the withdrawal for a failed component before its cleanup runs:
/// the cleanup can wait for seconds (a kernel change retried, a stalled
/// process), and the plane is withdrawing meanwhile, so its cause is known
/// to Status from the first moment on (`lastError`, `lastErrorDetail`).
pub(super) fn withdrawing(
    reports: &Publisher,
    problem: Problem,
    detail: Option<String>,
    apply: meshway_model::recovery::Health,
) -> super::super::Report {
    let mut report = reports.borrow().clone();
    report.plane = DataPlane::Failed;
    report.apply = apply;
    report.content = None;
    report.problem = Some(problem);
    report.problem_detail = detail;
    report.completion = Some(Err(failure(problem)));
    report.observed_at = chrono::Utc::now();
    report.facts = None;
    report.listeners = None;
    report.exit_flows = None;
    if let Some(capability) = &mut report.exit_capability {
        capability.set_installed(false);
    }
    reports.send_replace(report.clone());
    report
}

/// The receipt failure of a component's `problem`.
pub(super) fn failure(problem: Problem) -> ApplyFailure {
    match problem {
        Problem::Dns => ApplyFailure::Dns,
        Problem::Tun => ApplyFailure::Tun,
        Problem::Transport => ApplyFailure::Listeners,
        _ => ApplyFailure::Underlay,
    }
}

/// Why installed content was withdrawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cause {
    /// A required component failed (`problem`).
    Failure(Problem),
    /// The netmap lease ended.
    LeaseEnded,
    /// Control withdrew the lease.
    Withdrawn,
}

/// Installed content was withdrawn: why and when. Kept until content
/// installs again, whatever input triggers that install.
#[derive(Clone, Copy, Debug)]
pub(super) struct Withdrawn {
    pub cause: Cause,
    pub at: tokio::time::Instant,
}

impl Withdrawn {
    pub fn now(problem: Problem) -> Self {
        Self::of(Cause::Failure(problem))
    }
    pub fn of(cause: Cause) -> Self {
        Self {
            cause,
            at: tokio::time::Instant::now(),
        }
    }
    /// The one log line of the reinstall that ends this withdrawal, at the
    /// point the content is installed again; `trigger` names the input that
    /// led there (an offer, a retry, a component's recovery).
    pub fn reinstalled(self, trigger: &'static str, serial: u64, complete: bool) {
        let withdrawn_ms = self.at.elapsed().as_millis() as u64;
        match self.cause {
            Cause::Failure(Problem::Dns) => tracing::info!(
                trigger,
                withdrawn_ms,
                netmap_serial = serial,
                complete,
                "Required DNS component recovered; netmap reinstalled"
            ),
            Cause::Failure(problem) => tracing::info!(
                ?problem,
                trigger,
                withdrawn_ms,
                netmap_serial = serial,
                complete,
                "Required packet plane component recovered; netmap reinstalled"
            ),
            Cause::LeaseEnded | Cause::Withdrawn => tracing::info!(
                cause = if self.cause == Cause::LeaseEnded {
                    "leaseEnded"
                } else {
                    "netmapWithdrawn"
                },
                trigger,
                withdrawn_ms,
                netmap_serial = serial,
                complete,
                "Netmap installed again after its withdrawal"
            ),
        }
    }
}
