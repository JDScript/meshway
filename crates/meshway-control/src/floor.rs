//! `clock.json`: the UTC high-water mark this Node has observed, so that a
//! wall clock set back across a restart cannot revive an expired lease.
//!
//! ```json
//! { "format": "meshway-clock-floor/1", "highWaterMs": 1791654625926 }
//! ```
//!
//! Written at most once per [`STEP_MS`] and joined at shutdown. A missing
//! or unreadable file is no floor; the signed lease's `issuedAt` still is one.
use crate::{Control, storage};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub(crate) const FILE: &str = "clock.json";
const FORMAT: &str = "meshway-clock-floor/1";
/// The floor may lag the true high water by at most this much.
pub(crate) const STEP_MS: u64 = 60_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Floor {
    format: String,
    high_water_ms: u64,
}

pub(crate) fn load(directory: &meshway_private_storage::Directory) -> Option<u64> {
    let bytes = storage::read(directory, FILE).ok()??;
    let floor = storage::decode::<Floor>(&bytes).ok()?;
    (floor.format == FORMAT).then_some(floor.high_water_ms)
}

impl Control {
    /// The UTC high-water mark: never behind a time this Node has observed,
    /// including before a restart. Engine raises its lease clock to it.
    pub fn clock_floor_ms(&self) -> Option<u64> {
        self.clock.now().ok()
    }

    /// Save the high-water mark when it advanced by [`STEP_MS`]; the Control
    /// lock never waits for the write (at most one in flight).
    pub(crate) async fn persist_floor(&mut self) {
        let Ok(now) = self.clock.now() else {
            return;
        };
        if self
            .floor_saved
            .is_some_and(|saved| now < saved.saturating_add(STEP_MS))
        {
            return;
        }
        let Ok(bytes) = storage::encode(&Floor {
            format: FORMAT.to_owned(),
            high_water_ms: now,
        }) else {
            return;
        };
        if let Some(previous) = self.floor_write.take() {
            let _ = previous.await;
        }
        let directory = Arc::clone(&self.directory);
        self.floor_write = Some(tokio::task::spawn_blocking(move || {
            directory.replace(FILE, &bytes)
        }));
        self.floor_saved = Some(now);
    }

    pub(crate) async fn finish_floor_write(&mut self) {
        if let Some(write) = self.floor_write.take() {
            let _ = write.await;
        }
    }
}
