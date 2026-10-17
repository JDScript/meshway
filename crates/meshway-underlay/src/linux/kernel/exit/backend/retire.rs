//! Closing and retiring projections, reply-tuple quarantine and its
//! expiry, and reconciliation of a failed admitted operation.
use super::*;

impl Backend {
    pub async fn close_projection(
        &mut self,
        incarnation: super::super::ExitIncarnation,
    ) -> Result<()> {
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
    pub async fn retire(&mut self, incarnation: super::super::ExitIncarnation) -> Result<()> {
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
        // Any other flow has one projection per original tuple: a different
        // marked entry of it is this projection's own successor. The kernel
        // deleted the recorded one (masquerade, when the outer address it
        // translated to left this host on a roam) and the flow's next packet
        // created it, translated to the current outer address; both reply
        // tuples are quarantined and the successor is deleted.
        let echo = matches!(
            record.tuple.transport,
            super::super::ExitTransport::IcmpEcho { .. }
        );
        let (replaced, superseded) = match (&record.ct, &current) {
            (Some(old), Some(entry)) if !same_entry(old, entry) => {
                if !echo {
                    (false, Some(old.clone()))
                } else if self.attributable(Some(incarnation), record.tuple, entry) {
                    (true, None)
                } else {
                    return Err(Error::Conflict);
                }
            }
            _ => (false, None),
        };
        if let Some(old) = &superseded {
            self.quarantine_reply(incarnation, record.tuple, old)
                .await?;
        }
        // Preserve a confirmed translated tuple even if conntrack aged out
        // before retirement. Absence must not erase the old-reply tombstone.
        let entry = if replaced {
            record.ct.clone()
        } else {
            current.or(record.ct.clone())
        };
        if let Some(entry) = entry {
            self.quarantine_reply(incarnation, record.tuple, &entry)
                .await?;
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

    /// Quarantine `entry`'s reply tuple for `incarnation` and record it as
    /// the projection's entry (journal first, then the kernel set).
    async fn quarantine_reply(
        &mut self,
        incarnation: super::super::ExitIncarnation,
        tuple: OriginalTuple,
        entry: &conntrack::Entry,
    ) -> Result<()> {
        let retired = conntrack::RetiredTuple {
            tuple: entry.reply.clone(),
            sequence: tuple.sequence(),
            retired_ms: now_ms(),
        };
        if self.ledger.program.legacy {
            if !self.ledger.legacy_quarantine.contains(&entry.reply) {
                self.ledger.legacy_quarantine.push(entry.reply.clone());
            }
            self.store.persist(&self.ledger).await?;
            objects::mutate(&mut self.commands, &json!({"nftables": [self.ledger.program.element("add", "quarantine", vec![entry.reply.key()])]})).await?;
            return Ok(());
        }
        if !self
            .ledger
            .quarantine
            .iter()
            .any(|old| old.tuple == retired.tuple && old.sequence == retired.sequence)
        {
            if self.ledger.quarantine.len() + self.ledger.legacy_quarantine.len() >= HISTORY {
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
        Ok(())
    }

    /// Drop quarantined reply tuples retired at least `age` ago, oldest
    /// first; with `Duration::ZERO`, only the oldest one. The quarantine
    /// covers late replies of retired flows after conntrack deletion; an
    /// entry older than [`QUARANTINE_MIN_AGE`] has outlived every kernel
    /// timeout that could still deliver such a reply.
    ///
    /// The kernel sets are read first and only elements still present are
    /// deleted: a deletion the kernel applied whose answer was lost is
    /// completed by the journal write, never retried against nothing.
    pub(super) async fn expire_quarantine(&mut self, age: Duration) -> Result<()> {
        let expired = expired_quarantine(&self.ledger.quarantine, now_ms(), age)?;
        if expired.is_empty() {
            return Ok(());
        }
        let dump = objects::dump(&mut self.commands, &self.ledger.program).await?;
        let filter = self.ledger.program.filter();
        let elements: Vec<_> = self
            .ledger
            .program
            .quarantine_sets()
            .into_iter()
            .filter_map(|name| {
                let present = dump
                    .elements
                    .get(&format!("inet/{filter}/set/{name}"))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                let values: Vec<_> = expired
                    .iter()
                    .filter(|old| old.set() == name)
                    .map(conntrack::RetiredTuple::key)
                    .filter(|key| present.iter().any(|element| element_key(element) == key))
                    .collect();
                (!values.is_empty()).then(|| self.ledger.program.element("delete", &name, values))
            })
            .collect();
        // Kernel first: a crash before the journal write re-adds them.
        if !elements.is_empty() {
            objects::mutate(&mut self.commands, &json!({"nftables": elements})).await?;
        }
        self.ledger.quarantine.retain(|old| !expired.contains(old));
        self.store.persist(&self.ledger).await
    }
    /// Whether a marked conntrack `entry` of `tuple`'s echo connection
    /// belongs to this owner's projections: another unretired projection of
    /// the connection holds it or has not confirmed one yet. A projection
    /// recording a different entry no longer has it: the kernel keeps one
    /// entry per original tuple. An entry no projection can account for is
    /// foreign.
    pub(super) fn attributable(
        &self,
        except: Option<super::super::ExitIncarnation>,
        tuple: OriginalTuple,
        entry: &conntrack::Entry,
    ) -> bool {
        matches!(
            tuple.transport,
            super::super::ExitTransport::IcmpEcho { .. }
        ) && self.ledger.projections.iter().any(|flow| {
            Some(flow.incarnation) != except
                && !flow.retired
                && flow.tuple.same_connection(tuple)
                && flow.ct.as_ref().is_none_or(|ct| same_entry(ct, entry))
        })
    }
    /// Reconcile a failed/cancelled admitted operation before accepting another
    /// request. A lost caller cannot leave a positive kernel qualification.
    pub async fn failed(&mut self, incarnation: super::super::ExitIncarnation) -> Result<()> {
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
