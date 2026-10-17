//! Ending the packet plane: shutdown and lease withdrawal.
use super::super::{plane::Plane, reporting::Publisher};
use meshway_engine::Engine;
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
    report.problem_detail = None;
    report.completion = None;
    report.content = None;
    report.expires_at = None;
    report.facts = None;
    report.exit_flows = None;
    if let Some(capability) = &mut report.exit_capability {
        capability.set_installed(false);
    }
    reports.send_replace(report);
}
