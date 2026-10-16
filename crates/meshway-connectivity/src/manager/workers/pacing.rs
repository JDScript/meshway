//! Candidate pacing of one peer worker: the failure backoff of each dial
//! target and the changes that start every candidate over.
use super::*;

/// Longest pause between two attempts of one failing candidate, or between
/// two coordinated attempts this Node starts toward one peer. A peer that
/// restarted, or whose path recovered, is retried within this bound.
pub(in crate::manager) const MAX_CANDIDATE_BACKOFF: std::time::Duration =
    std::time::Duration::from_secs(60);

/// The pause after `failures` consecutive failures: `base · 2^(failures − 1)`,
/// at most [`MAX_CANDIDATE_BACKOFF`].
pub(in crate::manager) fn backoff(base: std::time::Duration, failures: u32) -> std::time::Duration {
    meshway_model::Backoff::new(base, MAX_CANDIDATE_BACKOFF).delay(failures)
}

/// One dial target: candidate key and endpoint.
pub(super) type TargetKey = (String, String, meshway_transport::spec::Port);

/// Candidate pacing of one peer worker. Every candidate is tried, whatever
/// its address class; a failed one waits `retry_backoff · 2^(failures − 1)`,
/// at most [`MAX_CANDIDATE_BACKOFF`], before its next attempt. A channel with
/// the peer (selected now, or installed since the last look), a changed
/// candidate set (the peer's endpoint facts), a change of this edge in the
/// content ([`Plan::edge`]) or of this Node's own mappings start every
/// candidate over. Other content changes leave the pacing as it is.
/// Addresses only say where to try: identity and admission stay with the
/// handshake.
#[derive(Default)]
pub(super) struct Pacing {
    edge: Option<crate::plan::Edge>,
    targets: std::collections::BTreeSet<TargetKey>,
    own: std::collections::BTreeSet<String>,
    /// Underlay path changes of this Node seen at the last sweep.
    network: u64,
    pub(super) last_link: Option<[u8; 32]>,
    failed: BTreeMap<TargetKey, (u32, tokio::time::Instant)>,
}

impl Pacing {
    pub(super) fn key(candidate: &Candidate, target: &Target) -> TargetKey {
        (
            candidate.key.clone(),
            target.endpoint.host.clone(),
            target.endpoint.port,
        )
    }

    /// Start over when the edge, the candidate set, this Node's own
    /// mappings or the peer's latest channel changed since the last sweep.
    /// Returns why paused candidates started over, if any did.
    pub(super) fn sweep(
        &mut self,
        edge: crate::plan::Edge,
        targets: std::collections::BTreeSet<TargetKey>,
        own: std::collections::BTreeSet<String>,
        network: u64,
        last_link: Option<[u8; 32]>,
    ) -> Option<&'static str> {
        let cause = if self.edge.as_ref() != Some(&edge) {
            Some("edge")
        } else if self.targets != targets {
            Some("candidates")
        } else if self.network != network {
            Some("own_network")
        } else if self.own != own {
            Some("own_mapping")
        } else if self.last_link != last_link {
            Some("channel")
        } else {
            None
        };
        let restarted = cause.filter(|_| !self.failed.is_empty());
        if cause.is_some() {
            self.failed.clear();
        }
        self.edge = Some(edge);
        self.targets = targets;
        self.own = own;
        self.network = network;
        self.last_link = last_link;
        restarted
    }

    /// A channel with the peer exists: its candidates start over later.
    pub(super) fn connected(&mut self, last_link: Option<[u8; 32]>) {
        self.failed.clear();
        self.last_link = last_link;
    }

    pub(super) fn due(&self, key: &TargetKey, now: tokio::time::Instant) -> bool {
        self.failed.get(key).is_none_or(|(_, at)| *at <= now)
    }

    /// Record a failed attempt; returns its consecutive failures and pause.
    pub(super) fn failed(
        &mut self,
        key: TargetKey,
        base: std::time::Duration,
        now: tokio::time::Instant,
    ) -> (u32, std::time::Duration) {
        let entry = self.failed.entry(key).or_insert((0, now));
        entry.0 = entry.0.saturating_add(1);
        let pause = backoff(base, entry.0);
        entry.1 = now + pause;
        (entry.0, pause)
    }

    /// When the earliest paused candidate of the current set is due again.
    pub(super) fn next_due(&self) -> Option<tokio::time::Instant> {
        self.targets
            .iter()
            .map(|key| self.failed.get(key).map(|(_, at)| *at))
            .min()
            .flatten()
    }
}
