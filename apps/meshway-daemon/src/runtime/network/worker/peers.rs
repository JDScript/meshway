/// Recomputes the peers' reported channels only when the facts or their
/// freshness change, not on every packet turn.
#[derive(Default)]
pub(super) struct PeerChannels {
    facts: Option<
        std::sync::Arc<
            std::collections::BTreeMap<meshway_model::NodeId, meshway_protocol::PeerFacts>,
        >,
    >,
    until: Option<chrono::DateTime<chrono::Utc>>,
}
impl PeerChannels {
    pub(super) fn changed(
        &mut self,
        known: Option<&meshway_control::KnownPeers>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<
        std::collections::BTreeMap<
            meshway_model::NodeId,
            std::collections::BTreeSet<meshway_model::NodeId>,
        >,
    > {
        let facts = known.map(|known| &known.peers);
        let same = match (&self.facts, facts) {
            (Some(old), Some(new)) => std::sync::Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if same && self.until.is_none_or(|until| now < until) {
            return None;
        }
        self.facts = facts.cloned();
        self.until = facts.and_then(|facts| {
            facts
                .values()
                .map(|facts| facts.expires_at)
                .filter(|at| *at > now)
                .min()
        });
        Some(known.map(|known| known.channels(now)).unwrap_or_default())
    }
}
