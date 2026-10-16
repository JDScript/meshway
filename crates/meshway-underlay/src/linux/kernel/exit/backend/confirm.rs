//! Confirmation and renewal of installed projections, and the read-back
//! of their nft set elements.
use super::*;

impl Backend {
    /// The owner's polling of [`Self::poll_confirm`], run to its end.
    #[cfg(test)]
    pub async fn confirm(
        &mut self,
        incarnation: super::super::ExitIncarnation,
    ) -> Result<CtConfirmation> {
        let until = self.confirmation_cutoff(incarnation)?;
        loop {
            if let Some(confirmation) = self.poll_confirm(incarnation, until).await? {
                return Ok(confirmation);
            }
            tokio::time::sleep_until((Instant::now() + CONFIRM_POLL).min(until).into()).await;
        }
    }

    /// The last instant an installed projection may still be confirmed.
    pub fn confirmation_cutoff(
        &self,
        incarnation: super::super::ExitIncarnation,
    ) -> Result<Instant> {
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
        incarnation: super::super::ExitIncarnation,
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
                // Not there yet, or a lookup that failed: unknown, not an
                // absence. Look again until the cutoff.
                Ok(Ok(None) | Err(_)) => return Ok(None),
                _ => {}
            }
        }
        self.close_projection(incarnation).await?;
        Ok(Some(CtConfirmation::ConfirmFailed { incarnation }))
    }
    pub async fn renew(
        &mut self,
        incarnation: super::super::ExitIncarnation,
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
            // A failed batch proves neither outcome. It is atomic: the element
            // is there (with its old or its new timeout), or gone with an
            // expired old one. While it reads back there, the same batch runs
            // once more: it sets the new timeout either way. A read that
            // fails proves nothing either; it is read again (three reads).
            let _ = self.commands.settle().await;
            let mut present = Err(Error::Netlink);
            for _ in 0..3 {
                present = self.present(&[old.tuple]).await;
                if !matches!(present, Err(Error::Netlink)) {
                    break;
                }
            }
            let again = matches!(present.as_deref(), Ok([true]))
                && objects::mutate(&mut self.commands, &batch).await.is_ok();
            if !again {
                self.close_projection(incarnation).await?;
                return Err(Error::Expired);
            }
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
    pub(super) async fn verify_elements(&mut self, tuples: &[OriginalTuple]) -> Result<()> {
        if self.present(tuples).await?.into_iter().all(|found| found) {
            Ok(())
        } else {
            Err(Error::Conflict)
        }
    }

    /// Whether each of `tuples` has exactly one element in its set, read from
    /// one dump that also matches the owned objects.
    pub(super) async fn present(&mut self, tuples: &[OriginalTuple]) -> Result<Vec<bool>> {
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

    pub(super) async fn verify_element(
        &mut self,
        tuple: OriginalTuple,
        present: bool,
    ) -> Result<()> {
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
}
