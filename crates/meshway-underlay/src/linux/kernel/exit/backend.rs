use super::super::verification::Verification;
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
    #[cfg(test)]
    pub async fn install(
        &mut self,
        incarnation: super::ExitIncarnation,
        tuple: OriginalTuple,
        deadline: Deadline,
        scope: ExitScope,
    ) -> Result<ProjectionAck> {
        let mut results = self
            .install_batch(vec![Install {
                incarnation,
                tuple,
                deadline,
                scope,
            }])
            .await;
        results.pop().unwrap_or(Err(Error::Worker))
    }

    /// Install the projections of one driver round. Each projection keeps
    /// everything an installation of its own requires: its ownership and
    /// conflict checks (journal, conntrack, special addresses), its journal
    /// record before any kernel mutation, its own element and its own check
    /// that the element is in the kernel before it is acknowledged. What the
    /// round shares is done once: the table ownership check, the journal
    /// writes, the nft transaction and the read-back dump. One result per
    /// projection, in order; a projection that fails does not change another
    /// one's result (a failed shared transaction is retried per projection).
    pub async fn install_batch(&mut self, items: Vec<Install>) -> Vec<Result<ProjectionAck>> {
        let started = Instant::now();
        let mut results: Vec<Option<Result<ProjectionAck>>> = items.iter().map(|_| None).collect();
        let mut accepted = Vec::new();
        for (index, item) in items.iter().enumerate() {
            if let Some(live) = self.live.get(&item.incarnation).cloned() {
                // A replayed request for a live projection: its own check.
                let result = async {
                    if live.tuple != item.tuple
                        || live.scope.binding != item.scope.binding
                        || !live.valid()
                    {
                        return Err(Error::Conflict);
                    }
                    self.verify_elements(&[item.tuple]).await?;
                    if !live.valid() {
                        return Err(Error::Expired);
                    }
                    Ok(ProjectionAck {
                        incarnation: item.incarnation,
                    })
                }
                .await;
                results[index] = Some(result);
                continue;
            }
            match self.admit(item, &accepted, &items).await {
                Ok(()) => accepted.push(index),
                Err(error) => results[index] = Some(Err(error)),
            }
        }
        if !accepted.is_empty() {
            let installed = self.install_accepted(&items, &accepted).await;
            for (index, result) in accepted.iter().zip(installed) {
                results[*index] = Some(result);
            }
            tracing::debug!(
                operation = "install",
                projections = accepted.len(),
                elapsed_us = started.elapsed().as_micros() as u64,
                "Exit projections installed"
            );
        }
        results
            .into_iter()
            .map(|result| result.unwrap_or(Err(Error::Worker)))
            .collect()
    }

    /// The checks of one projection that need no kernel mutation, against the
    /// journal and the projections accepted before it in the same round.
    async fn admit(&mut self, item: &Install, accepted: &[usize], items: &[Install]) -> Result<()> {
        if self.ledger.program.legacy {
            return Err(Error::Expired);
        }
        if self.stop.is_cancelled() || !item.deadline.valid() || item.scope.revoked.is_cancelled() {
            return Err(Error::Expired);
        }
        let pending = accepted.len();
        // Every projection must be retirable into the quarantine; aged
        // quarantine entries make room first, so history never blocks a new
        // flow beyond its revocation purpose.
        if self.ledger.quarantine.len()
            + self.ledger.legacy_quarantine.len()
            + self.ledger.projections.len()
            + pending
            >= HISTORY
        {
            self.expire_quarantine(QUARANTINE_MIN_AGE).await?;
        }
        if item.incarnation.is_zero()
            || self.ledger.projections.len() + pending >= HISTORY
            || self.live.len() + pending >= CAPACITY
            || self.ledger.quarantine.len()
                + self.ledger.legacy_quarantine.len()
                + self.ledger.projections.len()
                + pending
                >= HISTORY
        {
            return Err(Error::Capacity);
        }
        if self
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == item.incarnation || flow.tuple == item.tuple)
            || accepted.iter().any(|&other| {
                items[other].incarnation == item.incarnation || items[other].tuple == item.tuple
            })
        {
            return Err(Error::Conflict);
        }
        item.tuple
            .validate(&self.ledger.program.pools, &self.local)
            .map_err(|_| Error::Target)?;
        timeout(&item.deadline)?;
        Ok(())
    }

    /// The kernel part of [`Self::install_batch`] for admitted projections.
    async fn install_accepted(
        &mut self,
        items: &[Install],
        accepted: &[usize],
    ) -> Vec<Result<ProjectionAck>> {
        let all = |error: Error| {
            accepted
                .iter()
                .map(|_| Err(replicate(&error)))
                .collect::<Vec<_>>()
        };
        // Ownership of the tables and of every listed element: one dump for
        // the round, before anything of it is written.
        if let Err(error) = self.verify().await {
            return all(error);
        }
        let mut results: Vec<Option<Result<ProjectionAck>>> =
            accepted.iter().map(|_| None).collect();
        // A mark-owned entry without this incarnation's journal binding is not
        // adoptable either. Install replay above retains the existing binding.
        for (slot, &index) in accepted.iter().enumerate() {
            let tuple = items[index].tuple;
            let owned = match conntrack::lookup_owned(tuple, self.ledger.program.mark).await {
                Ok(owned) => owned,
                Err(_) => {
                    results[slot] = Some(Err(Error::Conflict));
                    continue;
                }
            };
            if let Some(entry) = owned {
                let shared = matches!(tuple.transport, super::ExitTransport::IcmpEcho { .. })
                    && self
                        .live
                        .values()
                        .any(|live| live.valid() && live.tuple.same_connection(tuple))
                    && self.attributable(None, tuple, &entry);
                if !shared {
                    results[slot] = Some(Err(Error::Conflict));
                }
            }
        }
        // Each projection's journal record precedes any kernel mutation; the
        // records of the round are written together.
        self.ledger.pending = true;
        for (slot, &index) in accepted.iter().enumerate() {
            if results[slot].is_some() {
                continue;
            }
            let item = &items[index];
            let Some(free) = (1..=HISTORY as u32).find(|slot| {
                !self
                    .ledger
                    .projections
                    .iter()
                    .any(|flow| flow.slot == *slot)
            }) else {
                results[slot] = Some(Err(Error::Capacity));
                continue;
            };
            self.ledger.projections.push(Projection {
                incarnation: item.incarnation,
                slot: free,
                tuple: item.tuple,
                expires_ms: item.deadline.expires_at_ms(),
                retired: false,
                ct: None,
            });
        }
        if let Err(error) = self.store.persist(&self.ledger).await {
            return results
                .into_iter()
                .map(|result| result.unwrap_or_else(|| Err(replicate(&error))))
                .collect();
        }
        // Routes and rules are per source/destination pair.
        let mut pairs: BTreeMap<(Ipv4Addr, Ipv4Addr), Result<()>> = BTreeMap::new();
        for (slot, &index) in accepted.iter().enumerate() {
            if results[slot].is_some() {
                continue;
            }
            let tuple = items[index].tuple;
            let pair = (tuple.source, tuple.destination);
            if let std::collections::btree_map::Entry::Vacant(vacant) = pairs.entry(pair) {
                let mut routed = self
                    .routing
                    .source(
                        tuple.source,
                        tuple.destination,
                        &mut self.ledger,
                        &mut self.store,
                    )
                    .await;
                if routed.is_ok() {
                    routed = self
                        .routing
                        .forwards(&self.ledger, tuple.source, tuple.destination)
                        .await;
                }
                vacant.insert(routed);
            }
            if let Some(Err(error)) = pairs.get(&pair) {
                results[slot] = Some(Err(replicate(error)));
            }
        }
        let pending: Vec<usize> = (0..accepted.len())
            .filter(|slot| results[*slot].is_none())
            .collect();
        let mut submitted = BTreeMap::new();
        let mut gates = BTreeMap::new();
        for &slot in &pending {
            match timeout(&items[accepted[slot]].deadline) {
                Ok(seconds) => {
                    gates.insert(slot, seconds);
                }
                Err(error) => results[slot] = Some(Err(error)),
            }
        }
        let elements: Vec<(usize, Value)> = gates
            .iter()
            .map(|(slot, seconds)| {
                let tuple = items[accepted[*slot]].tuple;
                (
                    *slot,
                    self.ledger
                        .program
                        .element("add", tuple.set(), vec![timed(tuple, *seconds)]),
                )
            })
            .collect();
        // One transaction for the round. It is atomic: when it fails nothing
        // of it was added, and each projection gets a transaction of its own.
        let now = Instant::now();
        let batch = json!({"nftables": elements.iter().map(|(_, element)| element.clone()).collect::<Vec<_>>()});
        if !elements.is_empty() {
            if objects::mutate(&mut self.commands, &batch).await.is_ok() {
                for (slot, _) in &elements {
                    submitted.insert(*slot, now);
                }
            } else {
                for (slot, element) in &elements {
                    let now = Instant::now();
                    match objects::mutate(&mut self.commands, &json!({"nftables": [element]})).await
                    {
                        Ok(_) => {
                            submitted.insert(*slot, now);
                        }
                        Err(error) => results[*slot] = Some(Err(error)),
                    }
                }
            }
        }
        // Every acknowledged projection's element is read back from the kernel.
        let tuples: Vec<OriginalTuple> = submitted
            .keys()
            .map(|slot| items[accepted[*slot]].tuple)
            .collect();
        match self.present(&tuples).await {
            Ok(present) => {
                for (slot, found) in submitted.keys().zip(present) {
                    if !found {
                        results[*slot] = Some(Err(Error::Conflict));
                    }
                }
            }
            Err(error) => {
                for slot in submitted.keys() {
                    results[*slot] = Some(Err(replicate(&error)));
                }
            }
        }
        for (slot, at) in &submitted {
            if results[*slot].is_some() {
                continue;
            }
            let item = &items[accepted[*slot]];
            let gate_until = *at + Duration::from_secs(gates[slot]);
            if Instant::now() >= gate_until
                || self.stop.is_cancelled()
                || item.scope.revoked.is_cancelled()
                || !item.deadline.valid()
            {
                results[*slot] = Some(match self.close_projection(item.incarnation).await {
                    Ok(()) => Err(Error::Expired),
                    Err(error) => Err(error),
                });
                continue;
            }
            let live = Live {
                tuple: item.tuple,
                deadline: item.deadline,
                scope: item.scope.clone(),
                retired: CancellationToken::new(),
                gate_until,
            };
            self.live.insert(item.incarnation, live.clone());
            match self.view.write() {
                Ok(mut view) => {
                    view.insert(item.incarnation, live);
                }
                Err(_) => {
                    results[*slot] = Some(Err(Error::Worker));
                    continue;
                }
            }
            results[*slot] = Some(Ok(ProjectionAck {
                incarnation: item.incarnation,
            }));
        }
        self.ledger.pending = false;
        if let Err(error) = self.store.persist(&self.ledger).await {
            for result in &mut results {
                if matches!(result, Some(Ok(_))) {
                    *result = Some(Err(replicate(&error)));
                }
            }
        }
        results
            .into_iter()
            .map(|result| result.unwrap_or(Err(Error::Worker)))
            .collect()
    }

    /// The owner's polling of [`Self::poll_confirm`], run to its end.
    #[cfg(test)]
    pub async fn confirm(&mut self, incarnation: super::ExitIncarnation) -> Result<CtConfirmation> {
        let until = self.confirmation_cutoff(incarnation)?;
        loop {
            if let Some(confirmation) = self.poll_confirm(incarnation, until).await? {
                return Ok(confirmation);
            }
            tokio::time::sleep_until((Instant::now() + CONFIRM_POLL).min(until).into()).await;
        }
    }

    /// The last instant an installed projection may still be confirmed.
    pub fn confirmation_cutoff(&self, incarnation: super::ExitIncarnation) -> Result<Instant> {
        let live = self.live.get(&incarnation).ok_or(Error::Expired)?;
        Ok((Instant::now() + CONFIRM_WAIT)
            .min(live.deadline.expires_at())
            .min(live.gate_until))
    }

    /// One bounded look for the projection's owned conntrack entry. `None`:
    /// not there yet, look again later; the caller (the single owner) keeps
    /// serving every other operation meanwhile. A forward packet that died
    /// before confirmation (TTL, MTU at this host) never confirms: at `until`
    /// the projection closes and the result is `ConfirmFailed`.
    pub async fn poll_confirm(
        &mut self,
        incarnation: super::ExitIncarnation,
        until: Instant,
    ) -> Result<Option<CtConfirmation>> {
        let Some(live) = self.live.get(&incarnation).cloned() else {
            return Ok(Some(CtConfirmation::ConfirmFailed { incarnation }));
        };
        if !self.stop.is_cancelled() && live.valid() && Instant::now() < until {
            match tokio::time::timeout_at(
                until.into(),
                conntrack::lookup_owned(live.tuple, self.ledger.program.mark),
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
                    if !record.ct.as_ref().is_some_and(|old| {
                        old.id != entry.id
                            || old.original != entry.original
                            || old.reply != entry.reply
                    }) {
                        record.ct = Some(entry.clone());
                        self.store.persist(&self.ledger).await?;
                        if live.valid() && !self.stop.is_cancelled() {
                            return Ok(Some(CtConfirmation::Confirmed {
                                incarnation,
                                conntrack_id: entry.id,
                            }));
                        }
                    }
                }
                Ok(Ok(None)) => return Ok(None),
                _ => {}
            }
        }
        self.close_projection(incarnation).await?;
        Ok(Some(CtConfirmation::ConfirmFailed { incarnation }))
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
        let entry = match conntrack::lookup_owned(old.tuple, self.ledger.program.mark).await {
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
    /// Each of `tuples` has exactly one element in its set (one dump).
    async fn verify_elements(&mut self, tuples: &[OriginalTuple]) -> Result<()> {
        if self.present(tuples).await?.into_iter().all(|found| found) {
            Ok(())
        } else {
            Err(Error::Conflict)
        }
    }

    /// Whether each of `tuples` has exactly one element in its set, read from
    /// one dump that also matches the owned objects.
    async fn present(&mut self, tuples: &[OriginalTuple]) -> Result<Vec<bool>> {
        if tuples.is_empty() {
            return Ok(Vec::new());
        }
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
        let expected = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if dump.objects != expected.objects || dump.handles != expected.handles {
            return Err(Error::Conflict);
        }
        Ok(tuples
            .iter()
            .map(|tuple| {
                let key = format!(
                    "inet/{}/set/{}",
                    self.ledger.program.filter(),
                    self.ledger.program.flow_set(*tuple)
                );
                dump.elements
                    .get(&key)
                    .into_iter()
                    .flatten()
                    .filter(|value| element_key(value) == &tuple.key())
                    .count()
                    == 1
            })
            .collect())
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
        // An abandoned nft operation is joined before the reconciliation
        // dump; a failed one is not an excuse to skip the negative
        // qualification transaction.
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
        // `install` persists the journal record before any kernel mutation. A
        // projection it rejected earlier (capacity, conflict, target) owns no
        // kernel state, so its retirement is complete and must not stop the
        // owner of every other flow.
        if !self
            .ledger
            .projections
            .iter()
            .any(|flow| flow.incarnation == incarnation)
        {
            return Ok(());
        }
        self.close_projection(incarnation).await?;
        let record = self
            .ledger
            .projections
            .iter()
            .find(|flow| flow.incarnation == incarnation)
            .ok_or(Error::Journal)?
            .clone();
        let current = conntrack::lookup_owned(record.tuple, self.ledger.program.mark)
            .await
            .map_err(|_| Error::Conflict)?;
        // An echo connection's entry that ended while this projection lived
        // (its ICMP timeout) was re-created by a later sequence: this
        // record's entry is gone (one entry per original tuple) and the
        // current one belongs to that projection, which deletes it in turn.
        let replaced = match (&record.ct, &current) {
            (Some(old), Some(entry)) if !same_entry(old, entry) => {
                if !self.attributable(Some(incarnation), record.tuple, entry) {
                    return Err(Error::Conflict);
                }
                true
            }
            _ => false,
        };
        // Preserve a confirmed translated tuple even if conntrack aged out
        // before retirement. Absence must not erase the old-reply tombstone.
        let entry = if replaced {
            record.ct.clone()
        } else {
            current.or(record.ct.clone())
        };
        if let Some(entry) = entry {
            let retired = conntrack::RetiredTuple {
                tuple: entry.reply.clone(),
                sequence: record.tuple.sequence(),
                retired_ms: now_ms(),
            };
            if self.ledger.program.legacy {
                if !self.ledger.legacy_quarantine.contains(&entry.reply) {
                    self.ledger.legacy_quarantine.push(entry.reply.clone());
                }
                self.store.persist(&self.ledger).await?;
                objects::mutate(&mut self.commands, &json!({"nftables": [self.ledger.program.element("add", "quarantine", vec![entry.reply.key()])]})).await?;
            } else {
                if !self
                    .ledger
                    .quarantine
                    .iter()
                    .any(|old| old.tuple == retired.tuple && old.sequence == retired.sequence)
                {
                    if self.ledger.quarantine.len() + self.ledger.legacy_quarantine.len() >= HISTORY
                    {
                        self.expire_quarantine(Duration::ZERO).await?;
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
            if !shared && !replaced {
                conntrack::delete(record.tuple, &entry, self.ledger.program.mark)
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
        // Conntrack is deleted, the reply tuple quarantined and routes
        // reclaimed: the record owns nothing more and leaves the history.
        self.ledger
            .projections
            .retain(|flow| flow.incarnation != incarnation);
        self.store.persist(&self.ledger).await
    }

    /// Drop quarantined reply tuples retired at least `age` ago, oldest
    /// first; with `Duration::ZERO`, only the oldest one. The quarantine
    /// covers late replies of retired flows after conntrack deletion; an
    /// entry older than [`QUARANTINE_MIN_AGE`] has outlived every kernel
    /// timeout that could still deliver such a reply.
    async fn expire_quarantine(&mut self, age: Duration) -> Result<()> {
        let expired = expired_quarantine(&self.ledger.quarantine, now_ms(), age)?;
        if expired.is_empty() {
            return Ok(());
        }
        let elements: Vec<_> = ["quarantine", "echo_quarantine"]
            .into_iter()
            .filter_map(|name| {
                let values: Vec<_> = expired
                    .iter()
                    .filter(|old| old.set() == name)
                    .map(conntrack::RetiredTuple::key)
                    .collect();
                (!values.is_empty()).then(|| self.ledger.program.element("delete", name, values))
            })
            .collect();
        // Kernel first: a crash before the journal write re-adds them.
        objects::mutate(&mut self.commands, &json!({"nftables": elements})).await?;
        self.ledger.quarantine.retain(|old| !expired.contains(old));
        self.store.persist(&self.ledger).await
    }
    /// Whether a marked conntrack `entry` of `tuple`'s echo connection
    /// belongs to this owner's projections: another unretired projection of
    /// the connection holds it or has not confirmed one yet. A projection
    /// recording a different entry no longer has it: the kernel keeps one
    /// entry per original tuple. An entry no projection can account for is
    /// foreign.
    fn attributable(
        &self,
        except: Option<super::ExitIncarnation>,
        tuple: OriginalTuple,
        entry: &conntrack::Entry,
    ) -> bool {
        matches!(tuple.transport, super::ExitTransport::IcmpEcho { .. })
            && self.ledger.projections.iter().any(|flow| {
                Some(flow.incarnation) != except
                    && !flow.retired
                    && flow.tuple.same_connection(tuple)
                    && flow.ct.as_ref().is_none_or(|ct| same_entry(ct, entry))
            })
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
        if self.ledger.quarantine.first().is_some_and(|old| {
            old.retired_ms <= now_ms().saturating_sub(QUARANTINE_AGE.as_millis() as u64)
        }) {
            self.expire_quarantine(QUARANTINE_AGE).await?;
        }
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

/// The quarantined tuples `expire_quarantine` drops, oldest first. With
/// `Duration::ZERO` the single oldest one, refused while it is younger than
/// [`QUARANTINE_MIN_AGE`]: the quarantine is then genuinely full.
fn expired_quarantine(
    quarantine: &[conntrack::RetiredTuple],
    now: u64,
    age: Duration,
) -> Result<Vec<conntrack::RetiredTuple>> {
    let millis = |duration: Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
    let cutoff = now.saturating_sub(millis(age));
    let mut expired: Vec<_> = quarantine
        .iter()
        .filter(|old| age.is_zero() || old.retired_ms <= cutoff)
        .cloned()
        .collect();
    expired.sort_by_key(|old| old.retired_ms);
    if age.is_zero() {
        expired.truncate(1);
        if expired
            .first()
            .is_some_and(|old| old.retired_ms > now.saturating_sub(millis(QUARANTINE_MIN_AGE)))
        {
            return Err(Error::Capacity);
        }
    }
    Ok(expired)
}

/// The same kernel conntrack entry (its id and both tuples).
fn same_entry(old: &conntrack::Entry, entry: &conntrack::Entry) -> bool {
    old.id == entry.id && old.original == entry.original && old.reply == entry.reply
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

#[cfg(test)]
mod tests {
    use super::*;

    fn retired(port: u16, retired_ms: u64) -> conntrack::RetiredTuple {
        conntrack::RetiredTuple {
            tuple: conntrack::Tuple {
                source: "192.0.2.1".parse().unwrap(),
                destination: "198.51.100.1".parse().unwrap(),
                protocol: 17,
                source_key: 53,
                destination_key: port,
            },
            sequence: 0,
            retired_ms,
        }
    }

    #[test]
    fn quarantine_history_ages_out_and_never_evicts_a_young_entry() {
        let min = QUARANTINE_MIN_AGE.as_millis() as u64;
        let now = 10 * min;
        let quarantine = vec![retired(3, now - 1), retired(1, 0), retired(2, now - min)];
        // Housekeeping keeps only what is still inside the age.
        let aged = expired_quarantine(&quarantine, now, QUARANTINE_MIN_AGE).unwrap();
        assert_eq!(aged, vec![retired(1, 0), retired(2, now - min)]);
        // A full quarantine yields its single oldest entry once it aged.
        assert_eq!(
            expired_quarantine(&quarantine, now, Duration::ZERO).unwrap(),
            vec![retired(1, 0)]
        );
        // A quarantine filled within the minimum age stays full: revocation
        // correctness outranks a new flow.
        assert!(matches!(
            expired_quarantine(&[retired(4, now - 1)], now, Duration::ZERO),
            Err(Error::Capacity)
        ));
    }
}
