//! Installing a batch of projections: admission, the shared nft and
//! conntrack steps, and per-projection results.
use super::*;

impl Backend {
    #[cfg(test)]
    pub async fn install(
        &mut self,
        incarnation: super::super::ExitIncarnation,
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
                let shared = matches!(
                    tuple.transport,
                    super::super::ExitTransport::IcmpEcho { .. }
                ) && self
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
}
