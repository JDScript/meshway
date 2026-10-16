use super::super::verification::{Reread, Verification};
use super::io::ExitCutoff as Deadline;
use super::{
    api::{CtConfirmation, ProjectionAck},
    child::Fence,
    command::Commands,
    conntrack,
    io::{ExitScope, Live, View},
    objects,
    program::{CAPACITY, HISTORY},
    routing::Routing,
    storage::{Ledger, Projection, Store},
    tuple::OriginalTuple,
};
use crate::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

mod confirm;
mod install;
mod retire;

/// One projection to install.
pub(super) struct Install {
    pub incarnation: super::ExitIncarnation,
    pub tuple: OriginalTuple,
    pub deadline: Deadline,
    pub scope: ExitScope,
}

pub(super) struct Backend {
    pub store: Store,
    pub ledger: Ledger,
    pub commands: Commands,
    pub routing: Routing,
    pub view: View,
    pub live: BTreeMap<super::ExitIncarnation, Live>,
    pub local: Vec<Ipv4Addr>,
    pub stop: CancellationToken,
}

impl Backend {
    pub async fn create(
        mut store: Store,
        ledger: Ledger,
        routing: Routing,
        view: View,
        local: Vec<Ipv4Addr>,
        stop: CancellationToken,
    ) -> Result<Self> {
        store.persist(&ledger).await?;
        let commands = Commands::recorded(Fence {
            directory: store.directory.clone(),
            installation: ledger.program.installation.clone(),
        });
        let mut backend = Self {
            store,
            ledger,
            commands,
            routing,
            view,
            live: BTreeMap::new(),
            local,
            stop,
        };
        backend.ledger.pending = true;
        backend.store.persist(&backend.ledger).await?;
        backend.ledger.receipt =
            Some(objects::mutate(&mut backend.commands, &backend.ledger.program.create()).await?);
        backend.store.persist(&backend.ledger).await?;
        backend.verify().await?;
        if !backend.ledger.quarantine.is_empty() {
            let elements: Vec<_> = ["quarantine", "echo_quarantine"]
                .into_iter()
                .filter_map(|name| {
                    let values: Vec<_> = backend
                        .ledger
                        .quarantine
                        .iter()
                        .filter(|tuple| tuple.set() == name)
                        .map(conntrack::RetiredTuple::key)
                        .collect();
                    (!values.is_empty())
                        .then(|| backend.ledger.program.element("add", name, values))
                })
                .collect();
            let batch = json!({"nftables": elements});
            objects::mutate(&mut backend.commands, &batch).await?;
        }
        if !backend.ledger.legacy_quarantine.is_empty() {
            let set = if backend.ledger.program.legacy {
                "quarantine"
            } else {
                "legacy_quarantine"
            };
            let batch = json!({"nftables": [backend.ledger.program.element("add", set, backend.ledger.legacy_quarantine.iter().map(conntrack::Tuple::key).collect())]});
            objects::mutate(&mut backend.commands, &batch).await?;
        }
        backend
            .routing
            .forward(&mut backend.ledger, &mut backend.store)
            .await?;
        backend
            .routing
            .related(&mut backend.ledger, &mut backend.store)
            .await?;
        backend.ledger.pending = false;
        backend.store.persist(&backend.ledger).await?;
        Ok(backend)
    }
    pub async fn verify(&mut self) -> Result<()> {
        self.check().await.result()
    }
    /// One read-back of the owned tables and projection elements against
    /// the receipt and journal, classified: a read that fails is unknown,
    /// never a change.
    pub async fn check(&mut self) -> Verification {
        if self.store.settle().await.is_err() {
            return Verification::Changed("journal_unwritten");
        }
        if self.commands.settle().await.is_err() {
            return Verification::Unavailable(Error::Netlink);
        }
        let Some(receipt) = self.ledger.receipt.as_ref() else {
            return Verification::Changed("receipt_missing");
        };
        let dump = match objects::read(&mut self.commands, &self.ledger.program).await {
            Ok(dump) => dump,
            Err(objects::Unread::Unavailable(error)) => return Verification::Unavailable(error),
            Err(objects::Unread::Changed(what)) => return Verification::Changed(what),
        };
        if dump.objects != receipt.objects || dump.handles != receipt.handles {
            return Verification::Changed("objects_changed");
        }
        let mut seen = Vec::new();
        for name in ["flows", "echo_flows"] {
            let key = format!("inet/{}/set/{name}", self.ledger.program.filter());
            for element in dump.elements.get(&key).into_iter().flatten() {
                let key = element_key(element);
                if seen.contains(&key)
                    || !self
                        .ledger
                        .projections
                        .iter()
                        .any(|flow| !flow.retired && flow.tuple.key() == *key)
                {
                    return Verification::Changed("flow_unowned");
                }
                seen.push(key);
            }
        }
        if self
            .live
            .values()
            .any(|live| live.valid() && !seen.iter().any(|key| **key == live.tuple.key()))
        {
            return Verification::Changed("flow_missing");
        }
        Verification::Holds
    }
    /// The read-back after a failed retirement of `incarnation`: the owned
    /// objects as [`Self::check`] reads them; the retirement is at its target
    /// once its journal record is gone (it is removed last), otherwise each
    /// step reads back and skips what is already done.
    pub async fn reread(&mut self, incarnation: super::ExitIncarnation) -> Reread {
        let done = !self
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == incarnation);
        Reread::of(self.check().await, done)
    }
    /// The read-back after failed housekeeping: the owned objects; each of
    /// its steps reads back and skips what is already done.
    pub async fn reread_maintain(&mut self) -> Reread {
        Reread::of(self.check().await, false)
    }
    pub async fn maintain(&mut self) -> Result<()> {
        if self.ledger.quarantine.first().is_some_and(|old| {
            old.retired_ms <= now_ms().saturating_sub(QUARANTINE_AGE.as_millis() as u64)
        }) {
            self.expire_quarantine(QUARANTINE_AGE).await?;
        }
        // Expired projections, and recorded ones no longer live: a
        // retirement that failed part way is completed here (each step reads
        // back and skips what is already done).
        let expired: std::collections::BTreeSet<_> = self
            .live
            .iter()
            .filter(|(_, live)| !live.valid())
            .map(|(id, _)| *id)
            .chain(
                self.ledger
                    .projections
                    .iter()
                    .filter(|flow| !self.live.contains_key(&flow.incarnation))
                    .map(|flow| flow.incarnation),
            )
            .collect();
        for incarnation in expired {
            self.retire(incarnation).await?;
        }
        Ok(())
    }
    pub async fn close(&mut self, monitored: bool) -> Result<()> {
        self.begin_close(monitored).await?;
        self.finish_close().await
    }
    /// Cleanup from what the kernel holds now: an earlier attempt may have
    /// removed part of it. Tables gone after a closing journal released
    /// every route, rule and projection are a completed removal; both tables
    /// present are closed again from the start (each step re-reads and
    /// skips what is already gone); anything else is not this owner's to
    /// remove.
    pub async fn close_observed(&mut self, monitored: bool) -> Result<()> {
        let _write = self.store.settle().await;
        let _command = self.commands.settle().await;
        match objects::present(&mut self.commands, &self.ledger.program).await? {
            0 if self.ledger.rules.is_empty()
                && self.ledger.routes.is_empty()
                && (self.ledger.projections.is_empty()
                    || (self.ledger.closing
                        && self.ledger.projections.iter().all(|flow| flow.retired))) =>
            {
                self.ledger.removed = true;
                self.ledger.pending = false;
                self.store.persist(&self.ledger).await
            }
            2 => self.close(monitored).await,
            _ => Err(Error::Conflict),
        }
    }
    pub async fn begin_close(&mut self, monitored: bool) -> Result<()> {
        #[cfg(test)]
        eprintln!("exit close: start");
        self.stop.cancel();
        for live in self.live.values() {
            live.retired.cancel();
        }
        self.view.write().map_err(|_| Error::Worker)?.clear();
        // Disk failure must not prevent a negative transaction on an already
        // durably owned table. Settle writers/tools and verify kernel identity
        // independently, close the gate, then persist/continue exact cleanup.
        let _write = self.store.settle().await;
        let _command = self.commands.settle().await;
        objects::verify(
            &mut self.commands,
            &self.ledger.program,
            self.ledger.receipt.as_ref().ok_or(Error::Journal)?,
        )
        .await?;
        #[cfg(test)]
        eprintln!("exit close: verified");
        self.ledger.closing = true;
        let _outcome = objects::mutate(&mut self.commands, &self.ledger.program.close_gate()).await;
        self.commands.settle().await.map_err(|_| Error::Netlink)?;
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
        let receipt = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        let prefix = format!("inet/{}/set", self.ledger.program.filter());
        let closed = dump.elements.get(&format!("{prefix}/closing"));
        if dump.objects != receipt.objects
            || dump.handles != receipt.handles
            || dump
                .elements
                .get(&format!("{prefix}/flows"))
                .is_some_and(|elements| !elements.is_empty())
            || dump
                .elements
                .get(&format!("{prefix}/echo_flows"))
                .is_some_and(|elements| !elements.is_empty())
            || !closed.is_some_and(|elements| {
                elements.len() == 2
                    && elements.contains(&json!(0))
                    && elements.contains(&json!(self.ledger.program.mark))
            })
        {
            return Err(Error::Conflict);
        }
        self.store.persist(&self.ledger).await?;
        #[cfg(test)]
        eprintln!("exit close: gate closed");
        for record in self.ledger.projections.clone() {
            if !record.retired {
                self.retire(record.incarnation).await?;
            }
        }
        self.routing
            .remove(&mut self.ledger, &mut self.store, monitored)
            .await?;
        #[cfg(test)]
        eprintln!("exit close: routes removed");
        Ok(())
    }
    pub async fn finish_close(&mut self) -> Result<()> {
        self.verify().await?;
        let _outcome = objects::mutate(&mut self.commands, &json!({"nftables": self.ledger.program.tables().into_iter().map(|(family, name)| json!({"delete": {"table": {"family": family, "name": name}}})).collect::<Vec<_>>() })).await;
        self.commands.settle().await.map_err(|_| Error::Netlink)?;
        if objects::present(&mut self.commands, &self.ledger.program).await? != 0 {
            return Err(Error::Conflict);
        }
        #[cfg(test)]
        eprintln!("exit close: tables removed");
        self.ledger.removed = true;
        self.ledger.pending = false;
        self.store.persist(&self.ledger).await
    }
}

/// A quarantined reply tuple is kept at least this long (above every
/// conntrack timeout of a deleted, NATed flow's late replies).
/// How long an installed projection waits for its first forwarded packet's
/// conntrack entry, and how often the owner looks while it waits.
pub(super) const CONFIRM_WAIT: Duration = Duration::from_secs(3);
pub(super) const CONFIRM_POLL: Duration = Duration::from_millis(10);
const QUARANTINE_MIN_AGE: Duration = Duration::from_secs(300);
/// Housekeeping drops quarantine entries older than this.
const QUARANTINE_AGE: Duration = Duration::from_secs(900);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// The same failure for each projection a shared step failed for.
pub(super) fn replicate(error: &Error) -> Error {
    match error {
        Error::Unsupported => Error::Unsupported,
        Error::Options => Error::Options,
        Error::Origin => Error::Origin,
        Error::ControlUnavailable => Error::ControlUnavailable,
        Error::Expired => Error::Expired,
        Error::Target => Error::Target,
        Error::Capacity => Error::Capacity,
        Error::Conflict => Error::Conflict,
        Error::Protection => Error::Protection,
        Error::JournalVersion { file, version } => Error::JournalVersion {
            file,
            version: *version,
        },
        Error::Resolver => Error::Resolver,
        Error::Netlink => Error::Netlink,
        Error::Drain => Error::Drain,
        Error::Worker => Error::Worker,
        Error::Io(error) | Error::Bind { source: error, .. } => {
            Error::Io(std::io::Error::new(error.kind(), error.to_string()))
        }
        Error::Journal | Error::Storage(_) => Error::Journal,
        Error::Tun(_) => Error::Worker,
    }
}

fn element_key(value: &Value) -> &Value {
    value
        .get("elem")
        .and_then(|element| element.get("val"))
        .unwrap_or(value)
}
fn timed(tuple: OriginalTuple, seconds: u64) -> Value {
    json!({"elem": {"val": tuple.key(), "timeout": seconds}})
}
fn timeout(deadline: &Deadline) -> Result<u64> {
    if !deadline.valid() {
        return Err(Error::Expired);
    }
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Expired)?
        .as_millis();
    let wall = u64::try_from(wall).map_err(|_| Error::Expired)?;
    let remaining = deadline
        .expires_at()
        .saturating_duration_since(Instant::now())
        .min(Duration::from_millis(
            deadline.expires_at_ms().saturating_sub(wall),
        ));
    remaining
        .checked_sub(super::command::BUDGET + Duration::from_secs(1))
        .map(|duration| duration.as_secs())
        .filter(|seconds| *seconds > 0)
        .ok_or(Error::Expired)
}
