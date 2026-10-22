//! Bounded reinstallation of owned capture objects removed outside their
//! owner, and the monitor's view of the own rules.
use super::*;

/// The recovery of a repair whose reads failed: read again every
/// [`REPAIR_RETRY`], handed to the Host as unavailable after
/// [`REPAIR_READS_LIMIT`] from the first failure.
fn owed_repair() -> meshway_model::recovery::Recovery {
    meshway_model::recovery::Recovery::new(meshway_model::recovery::Policy {
        backoff: meshway_model::Backoff::new(REPAIR_RETRY, REPAIR_RETRY),
        horizon: meshway_model::recovery::Horizon::Escalate(REPAIR_READS_LIMIT),
        reread_blocked: None,
    })
}

impl Kernel {
    /// The monitor's view of the capture block's own rules follows the journal.
    pub(super) fn publish_own_rules(&self) -> Result<()> {
        self.expected.lock().map_err(|_| Error::Worker)?.own_rules = self
            .journal
            .rules
            .iter()
            .map(|record| record.observed.clone())
            .collect();
        Ok(())
    }

    /// Reinstall journal rules and capture-table routes that something else
    /// removed (a network manager cleaning up rules it does not know), in
    /// the order the block needs them: end and bypass before the pool rules,
    /// each pool's sink before its lookup. A slot another owner holds now is
    /// a conflict; so is a fourth repair within a minute.
    ///
    /// A repair whose reads fail cannot tell what is missing: it stays owed
    /// ([`Self::repair_at`], [`REPAIR_RETRY`] apart, not counted as a
    /// repair) and once its reads failed for [`REPAIR_READS_LIMIT`] ends
    /// ownership as unavailable (the Host re-proves it). No outcome leaves
    /// the owner looking healthy with a missing object it was told about.
    pub async fn repair(&mut self) -> Result<()> {
        let now = std::time::Instant::now();
        while self
            .repaired
            .front()
            .is_some_and(|at| now.duration_since(*at) >= REPAIR_WINDOW)
        {
            self.repaired.pop_front();
        }
        if self.repaired.len() >= MAX_REPAIRS {
            tracing::warn!("Owned capture objects keep being removed; ownership ends");
            self.repair_owed = None;
            self.shared.end(crate::OwnershipEnd::Conflict {
                check: "repair_limit",
            });
            return Err(Error::Conflict);
        }
        let (missing, lost) = match self.repair_reads().await {
            Ok(found) => found,
            Err(error) if kind(&error) == Kind::Unknown => {
                let owed = self.repair_owed.get_or_insert_with(owed_repair);
                match owed.observe(
                    meshway_model::recovery::Observation::unknown("repair_unavailable", &error),
                    now,
                ) {
                    meshway_model::recovery::Step::Escalate(_) => {
                        self.repair_owed = None;
                        self.end_after("repair", &error);
                    }
                    _ => {
                        let failures = owed.attempts();
                        if owed.transitioned() || failures.is_power_of_two() {
                            tracing::warn!(%error, failures, "Owned capture objects unreadable; the repair is retried");
                        }
                    }
                }
                return Err(error);
            }
            Err(error) => {
                self.repair_owed = None;
                self.end_after("repair", &error);
                return Err(error);
            }
        };
        self.repair_owed = None;
        self.repaired.push_back(now);
        tracing::warn!(
            rules = missing.len(),
            routes = lost.len(),
            "Reinstalling owned capture objects removed outside their owner"
        );
        let result = async {
            for record in &missing {
                self.objects().add_rule(record.spec.message()).await?;
            }
            for record in &lost {
                self.objects().add_route(record.spec.message()?).await?;
            }
            self.quiesce().await?;
            self.verify().await
        }
        .await;
        if let Err(error) = &result {
            self.end_after("repair", error);
        }
        result
    }

    /// When an owed repair ([`Self::repair`]) is due again.
    pub fn repair_at(&self) -> Option<std::time::Instant> {
        match self.repair_owed.as_ref()?.health() {
            meshway_model::recovery::Health::Retrying(retrying)
                if self.shared.ended().is_none() =>
            {
                Some(retrying.retry_at)
            }
            _ => None,
        }
    }

    /// The journal's rules (in reinstall order) and capture routes the
    /// kernel no longer holds.
    async fn repair_reads(&mut self) -> Result<(Vec<RuleRecord>, Vec<RouteRecord>)> {
        self.ready().await?;
        // A goto rule whose target was removed carries the kernel's
        // UNRESOLVED state flag: still the owner's own rule, not missing.
        let present: Vec<_> = get_rules(&self.handle, self.options.max_kernel_objects)
            .await?
            .into_iter()
            .map(|mut rule| {
                rule.header
                    .flags
                    .remove(rtnetlink::packet_route::rule::RuleFlags::Unresolved);
                encode_rule(rule)
            })
            .collect();
        let mut missing: Vec<_> = self
            .journal
            .rules
            .iter()
            .filter(|record| !present.contains(&record.observed))
            .cloned()
            .collect();
        missing.sort_by_key(|record| match record.spec.kind {
            RuleKind::End => 0,
            RuleKind::Bypass { .. } => 1,
            RuleKind::Sink { .. } => 2,
            RuleKind::Lookup { .. } => 3,
        });
        let routes: Vec<_> = get_routes(&self.handle, self.options.max_kernel_objects)
            .await?
            .into_iter()
            .map(encode_route)
            .collect();
        let lost = self
            .journal
            .routes
            .iter()
            .filter(|record| !routes.contains(&record.observed))
            .cloned()
            .collect();
        Ok((missing, lost))
    }
}
