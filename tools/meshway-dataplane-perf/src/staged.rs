//! Staged migration only, removed with the Underlay component migration:
//! the Host still installs a snapshot. This benchmark has no Server, so it
//! binds the static content to a fixed one-day lease of its own.
use crate::Error;
use meshway_authority::{DataPlaneKeys, ExecutionSnapshot, StagedIssuer};
use meshway_engine::Desired;

pub(crate) fn snapshot(
    desired: &Desired,
    keys: &DataPlaneKeys,
) -> Result<ExecutionSnapshot, Error> {
    let now = meshway_authority::now()?;
    let lease = meshway_model::NetmapLease::new(desired.reference(), now, now + 86_400_000)
        .map_err(|_| "fixture lease")?;
    let identity = StagedIssuer::new()?.issue(desired.content(), lease, None, keys)?;
    Ok(identity.snapshot().clone())
}
