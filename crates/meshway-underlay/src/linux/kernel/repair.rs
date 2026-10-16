//! Bounded reinstallation of owned capture objects removed outside their
//! owner, and the monitor's view of the own rules.
use super::*;

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
    /// repair) and after [`MAX_REPAIR_READS`] failed reads ends ownership as
    /// unavailable. No outcome leaves the owner looking healthy with a
    /// missing object it was told about.
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
            Err(error @ (Error::Netlink | Error::Io(_))) => {
                let failures = self.repair_owed.map_or(0, |(_, failures)| failures) + 1;
                if failures < MAX_REPAIR_READS {
                    tracing::warn!(%error, failures, "Owned capture objects unreadable; the repair is retried");
                    self.repair_owed = Some((now + REPAIR_RETRY, failures));
                } else {
                    self.repair_owed = None;
                    self.end_after("repair", &error);
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
        self.repair_owed
            .filter(|_| self.shared.ended().is_none())
            .map(|(at, _)| at)
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
