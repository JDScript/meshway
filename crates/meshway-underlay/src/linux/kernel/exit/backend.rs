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
use meshway_authority::Deadline;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

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
        backend.ledger.pending = false;
        backend.store.persist(&backend.ledger).await?;
        Ok(backend)
    }
    pub async fn verify(&mut self) -> Result<()> {
        self.store.settle().await?;
        self.commands.settle().await.map_err(|_| Error::Netlink)?;
        objects::verify(
            &mut self.commands,
            &self.ledger.program,
            self.ledger.receipt.as_ref().ok_or(Error::Journal)?,
        )
        .await?;
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
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
                    return Err(Error::Conflict);
                }
                seen.push(key);
            }
        }
        if self
            .live
            .values()
            .any(|live| live.valid() && !seen.iter().any(|key| **key == live.tuple.key()))
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub async fn install(
        &mut self,
        incarnation: super::ExitIncarnation,
        tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        if self.ledger.program.legacy {
            return Err(Error::Expired);
        }
        if self.stop.is_cancelled() || !deadline.valid() || scope.revoked.is_cancelled() {
            return Err(Error::Expired);
        }
        if let Some(live) = self.live.get(&incarnation).cloned() {
            if live.tuple != tuple || live.scope.binding != scope.binding || !live.valid() {
                return Err(Error::Conflict);
            }
            self.verify_element(tuple, true).await?;
            if !live.valid() {
                return Err(Error::Expired);
            }
            return Ok(ProjectionAck { incarnation });
        }
        if incarnation.is_zero()
            || self.ledger.projections.len() >= HISTORY
            || self.live.len() >= CAPACITY
            || self.ledger.quarantine.len()
                + self.ledger.legacy_quarantine.len()
                + self
                    .ledger
                    .projections
                    .iter()
                    .filter(|flow| !flow.retired)
                    .count()
                >= HISTORY
        {
            return Err(Error::Capacity);
        }
        if self
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == incarnation || flow.tuple == tuple)
        {
            return Err(Error::Conflict);
        }
        tuple
            .validate(&self.ledger.program.pools, &self.local)
            .map_err(|_| Error::Target)?;
        timeout(&deadline)?;
        self.verify().await?;
        // A mark-owned entry without this incarnation's journal binding is not
        // adoptable either. Install replay above retains the existing binding.
        if let Some(entry) =
            conntrack::lookup_owned(&mut self.commands, tuple, self.ledger.program.mark)
                .await
                .map_err(|_| Error::Conflict)?
        {
            let shared = matches!(tuple.transport, super::ExitTransport::IcmpEcho { .. })
                && self
                    .live
                    .values()
                    .any(|live| live.valid() && live.tuple.same_connection(tuple))
                && self
                    .ledger
                    .projections
                    .iter()
                    .filter(|flow| !flow.retired && flow.tuple.same_connection(tuple))
                    .all(|flow| flow.ct.as_ref().is_none_or(|ct| ct == &entry));
            if !shared {
                return Err(Error::Conflict);
            }
        }
        self.ledger.pending = true;
        self.ledger.projections.push(Projection {
            incarnation,
            slot: (self.ledger.projections.len() + 1) as u32,
            tuple,
            expires_ms: deadline.expires_at_ms(),
            retired: false,
            ct: None,
        });
        self.store.persist(&self.ledger).await?;
        self.routing
            .source(
                tuple.source,
                tuple.destination,
                &mut self.ledger,
                &mut self.store,
            )
            .await?;
        let seconds = timeout(&deadline)?;
        let submitted = Instant::now();
        let batch = json!({"nftables": [self.ledger.program.element("add", tuple.set(), vec![timed(tuple, seconds)])]});
        objects::mutate(&mut self.commands, &batch).await?;
        self.verify_element(tuple, true).await?;
        let gate_until = submitted + Duration::from_secs(seconds);
        if Instant::now() >= gate_until
            || self.stop.is_cancelled()
            || scope.revoked.is_cancelled()
            || !deadline.valid()
        {
            self.close_projection(incarnation).await?;
            return Err(Error::Expired);
        }
        let live = Live {
            tuple,
            deadline,
            scope,
            retired: CancellationToken::new(),
            gate_until,
        };
        self.live.insert(incarnation, live.clone());
        self.view
            .write()
            .map_err(|_| Error::Worker)?
            .insert(incarnation, live);
        self.ledger.pending = false;
        self.store.persist(&self.ledger).await?;
        Ok(ProjectionAck { incarnation })
    }
    pub async fn confirm(&mut self, incarnation: super::ExitIncarnation) -> Result<CtConfirmation> {
        let live = self.live.get(&incarnation).cloned().ok_or(Error::Expired)?;
        let until = (Instant::now() + Duration::from_secs(3))
            .min(live.deadline.expires_at())
            .min(live.gate_until);
        loop {
            if self.stop.is_cancelled() || !live.valid() || Instant::now() >= until {
                break;
            }
            match tokio::time::timeout_at(
                until.into(),
                conntrack::lookup_owned(&mut self.commands, live.tuple, self.ledger.program.mark),
            )
            .await
            {
                Ok(Ok(Some(entry)))
                    if live.valid() && !self.stop.is_cancelled() && Instant::now() < until =>
                {
                    let record = self
                        .ledger
                        .projections
                        .iter_mut()
                        .find(|flow| flow.incarnation == incarnation)
                        .ok_or(Error::Journal)?;
                    if record.ct.as_ref().is_some_and(|old| {
                        old.id != entry.id
                            || old.original != entry.original
                            || old.reply != entry.reply
                    }) {
                        break;
                    }
                    record.ct = Some(entry.clone());
                    self.store.persist(&self.ledger).await?;
                    if !live.valid() || self.stop.is_cancelled() {
                        break;
                    }
                    return Ok(CtConfirmation::Confirmed {
                        incarnation,
                        conntrack_id: entry.id,
                    });
                }
                Ok(Ok(None)) => {
                    tokio::time::sleep_until(
                        (Instant::now() + Duration::from_millis(10))
                            .min(until)
                            .into(),
                    )
                    .await
                }
                _ => break,
            }
        }
        self.close_projection(incarnation).await?;
        Ok(CtConfirmation::ConfirmFailed { incarnation })
    }
    pub async fn renew(
        &mut self,
        incarnation: super::ExitIncarnation,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        let old = self.live.get(&incarnation).cloned().ok_or(Error::Expired)?;
        if !old.valid()
            || old.scope.binding != scope.binding
            || scope.revoked.is_cancelled()
            || self.stop.is_cancelled()
        {
            return Err(Error::Expired);
        }
        let before = self
            .ledger
            .projections
            .iter()
            .find(|flow| flow.incarnation == incarnation)
            .and_then(|flow| flow.ct.clone())
            .ok_or(Error::Expired)?;
        self.verify().await?;
        let entry =
            match conntrack::lookup_owned(&mut self.commands, old.tuple, self.ledger.program.mark)
                .await
            {
                Ok(entry) => entry,
                Err(_) => {
                    self.close_projection(incarnation).await?;
                    return Err(Error::Conflict);
                }
            };
        if entry.as_ref().is_none_or(|entry| {
            entry.id != before.id
                || entry.original != before.original
                || entry.reply != before.reply
        }) {
            self.close_projection(incarnation).await?;
            return Err(Error::Conflict);
        }
        let seconds = timeout(&deadline)?;
        self.ledger.pending = true;
        self.store.persist(&self.ledger).await?;
        let seconds = seconds.min(timeout(&deadline)?);
        if !old.valid() {
            self.close_projection(incarnation).await?;
            return Err(Error::Expired);
        }
        let submitted = Instant::now();
        // An expired old element makes DELETE fail and aborts the entire batch.
        let batch = json!({"nftables": [self.ledger.program.element("delete", old.tuple.set(), vec![old.tuple.key()]), self.ledger.program.element("add", old.tuple.set(), vec![timed(old.tuple, seconds)])]});
        if objects::mutate(&mut self.commands, &batch).await.is_err() {
            self.close_projection(incarnation).await?;
            return Err(Error::Expired);
        }
        self.verify_element(old.tuple, true).await?;
        let gate_until = submitted + Duration::from_secs(seconds);
        if self.stop.is_cancelled()
            || !old.valid()
            || scope.revoked.is_cancelled()
            || !deadline.valid()
            || Instant::now() >= gate_until
        {
            self.close_projection(incarnation).await?;
            return Err(Error::Expired);
        }
        let live = Live {
            tuple: old.tuple,
            deadline,
            scope,
            retired: old.retired,
            gate_until,
        };
        self.ledger
            .projections
            .iter_mut()
            .find(|flow| flow.incarnation == incarnation)
            .ok_or(Error::Journal)?
            .expires_ms = live.deadline.expires_at_ms();
        self.live.insert(incarnation, live.clone());
        self.view
            .write()
            .map_err(|_| Error::Worker)?
            .insert(incarnation, live);
        self.ledger.pending = false;
        self.store.persist(&self.ledger).await?;
        Ok(ProjectionAck { incarnation })
    }
    async fn verify_element(&mut self, tuple: OriginalTuple, present: bool) -> Result<()> {
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
        let expected = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if dump.objects != expected.objects || dump.handles != expected.handles {
            return Err(Error::Conflict);
        }
        let key = format!(
            "inet/{}/set/{}",
            self.ledger.program.filter(),
            self.ledger.program.flow_set(tuple)
        );
        let matches = dump
            .elements
            .get(&key)
            .into_iter()
            .flatten()
            .filter(|value| element_key(value) == &tuple.key())
            .count();
        if matches != usize::from(present) {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    pub async fn close_projection(&mut self, incarnation: super::ExitIncarnation) -> Result<()> {
        if let Some(live) = self.live.remove(&incarnation) {
            live.retired.cancel();
        }
        self.view
            .write()
            .map_err(|_| Error::Worker)?
            .remove(&incarnation);
        let tuple = self
            .ledger
            .projections
            .iter()
            .find(|flow| flow.incarnation == incarnation)
            .ok_or(Error::Target)?
            .tuple;
        self.ledger.pending = true;
        self.store.persist(&self.ledger).await?;
        // A timed-out confirmation may have abandoned a read. Join it before
        // the reconciliation dump; a failed read is not an excuse to skip the
        // negative qualification transaction.
        let _outcome = self.commands.settle().await;
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
        let expected = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if dump.objects != expected.objects || dump.handles != expected.handles {
            return Err(Error::Conflict);
        }
        let key = format!(
            "inet/{}/set/{}",
            self.ledger.program.filter(),
            self.ledger.program.flow_set(tuple)
        );
        if dump
            .elements
            .get(&key)
            .into_iter()
            .flatten()
            .any(|value| element_key(value) == &tuple.key())
        {
            let _result = objects::mutate(&mut self.commands, &json!({"nftables": [self.ledger.program.element("delete", tuple.set(), vec![tuple.key()])]})).await;
        }
        self.verify_element(tuple, false).await?;
        self.ledger.pending = false;
        self.store.persist(&self.ledger).await
    }
    pub async fn retire(&mut self, incarnation: super::ExitIncarnation) -> Result<()> {
        self.close_projection(incarnation).await?;
        let record = self
            .ledger
            .projections
            .iter()
            .find(|flow| flow.incarnation == incarnation)
            .ok_or(Error::Journal)?
            .clone();
        let entry =
            conntrack::lookup_owned(&mut self.commands, record.tuple, self.ledger.program.mark)
                .await
                .map_err(|_| Error::Conflict)?;
        // Preserve a confirmed translated tuple even if conntrack aged out
        // before retirement. Absence must not erase the old-reply tombstone.
        if let Some(entry) = entry.or(record.ct.clone()) {
            if record.ct.as_ref().is_some_and(|old| {
                old.id != entry.id || old.reply != entry.reply || old.original != entry.original
            }) {
                return Err(Error::Conflict);
            }
            let retired = conntrack::RetiredTuple {
                tuple: entry.reply.clone(),
                sequence: record.tuple.sequence(),
            };
            if self.ledger.program.legacy {
                if !self.ledger.legacy_quarantine.contains(&entry.reply) {
                    self.ledger.legacy_quarantine.push(entry.reply.clone());
                }
                self.store.persist(&self.ledger).await?;
                objects::mutate(&mut self.commands, &json!({"nftables": [self.ledger.program.element("add", "quarantine", vec![entry.reply.key()])]})).await?;
            } else {
                if !self.ledger.quarantine.contains(&retired) {
                    if self.ledger.quarantine.len() + self.ledger.legacy_quarantine.len() >= HISTORY
                    {
                        return Err(Error::Capacity);
                    }
                    self.ledger.quarantine.push(retired.clone());
                }
                self.ledger
                    .projections
                    .iter_mut()
                    .find(|flow| flow.incarnation == incarnation)
                    .ok_or(Error::Journal)?
                    .ct = Some(entry.clone());
                self.store.persist(&self.ledger).await?;
                objects::mutate(&mut self.commands, &json!({"nftables": [self.ledger.program.element("add", retired.set(), vec![retired.key()])]})).await?;
            }
            let shared = self
                .live
                .values()
                .any(|live| live.valid() && live.tuple.same_connection(record.tuple));
            if !shared {
                conntrack::delete(
                    &mut self.commands,
                    record.tuple,
                    &entry,
                    self.ledger.program.mark,
                )
                .await
                .map_err(|_| Error::Netlink)?;
            }
        }
        self.ledger
            .projections
            .iter_mut()
            .find(|flow| flow.incarnation == incarnation)
            .ok_or(Error::Journal)?
            .retired = true;
        self.store.persist(&self.ledger).await?;
        if !self.ledger.closing {
            self.routing
                .reclaim(record.tuple, &mut self.ledger, &mut self.store)
                .await?;
        }
        Ok(())
    }
    /// Reconcile a failed/cancelled admitted operation before accepting another
    /// request. A lost caller cannot leave a positive kernel qualification.
    pub async fn failed(&mut self, incarnation: super::ExitIncarnation) -> Result<()> {
        if self
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == incarnation)
        {
            self.retire(incarnation).await?;
        }
        Ok(())
    }
    pub async fn maintain(&mut self) -> Result<()> {
        let expired: Vec<_> = self
            .live
            .iter()
            .filter(|(_, live)| !live.valid())
            .map(|(id, _)| *id)
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
