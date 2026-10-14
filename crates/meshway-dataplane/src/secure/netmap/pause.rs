use super::super::{View, crypto};
use super::FirstPackets;
use meshway_model::NodeId;
use std::time::Instant;

/// Paths whose last origin handshake ended unanswered.
const MAX_UNANSWERED: usize = 256;
/// Longest pause of a path whose handshakes keep ending unanswered.
pub const MAX_PATH_PAUSE: std::time::Duration = std::time::Duration::from_secs(60);

/// What a path's reachability depends on in the content: the key set of its
/// Nodes and the carriers between each hop. A pause outlives content changes
/// that leave it equal (another Node's remark, Access, DNS, exits).
#[derive(Clone, PartialEq, Eq, Debug)]
pub(in crate::secure) struct PathContent {
    keys: meshway_model::Digest,
    hops: Vec<(Vec<String>, Vec<String>)>,
}

impl PathContent {
    pub(in crate::secure) fn of(
        decisions: &mut crate::netmap::Decisions,
        reading: &View<'_>,
        path: &[NodeId],
    ) -> Option<Self> {
        Some(Self {
            keys: crypto::key_digest(reading, path).ok()?,
            hops: decisions.hop_carriers(reading.netmap(), path)?,
        })
    }
}

/// One paused path: consecutive unanswered handshakes, the end of the pause
/// and the content it was paused on.
pub(super) struct Paused {
    count: u32,
    until: Instant,
    content: Option<PathContent>,
}

impl FirstPackets {
    /// An origin handshake on `path` ended at its deadline without an
    /// answer: a relay could not reach the next hop, or the path is dead.
    /// The pause doubles from `base` up to [`MAX_PATH_PAUSE`]. A pause that
    /// ended only makes the path eligible again: its count stays, so the
    /// next unanswered handshake pauses it longer. The count resets on an
    /// answer, a change of the path's own content, or bounded collection
    /// (the entry whose pause ended longest ago gives way to a new path).
    pub(in crate::secure) fn unanswered(
        &mut self,
        path: &[NodeId],
        content: Option<PathContent>,
        now: Instant,
        base: std::time::Duration,
    ) {
        if self.unanswered.len() >= MAX_UNANSWERED && !self.unanswered.contains_key(path) {
            let Some(oldest) = self
                .unanswered
                .iter()
                .filter(|(_, paused)| paused.until <= now)
                .min_by_key(|(_, paused)| paused.until)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.unanswered.remove(&oldest);
        }
        let entry = self.unanswered.entry(path.to_vec()).or_insert(Paused {
            count: 0,
            until: now,
            content: content.clone(),
        });
        if entry.content != content {
            entry.count = 0;
            entry.content = content;
        }
        entry.count = entry.count.saturating_add(1);
        entry.until = now
            + base
                .saturating_mul(1 << (entry.count - 1).min(16))
                .min(MAX_PATH_PAUSE);
    }
    /// New content keeps the pause of every path whose own content is equal.
    pub(super) fn retain_paths(
        &mut self,
        mut content: impl FnMut(&[NodeId]) -> Option<PathContent>,
    ) {
        self.unanswered
            .retain(|path, paused| paused.content.is_some() && content(path) == paused.content);
    }
    pub fn answered(&mut self, path: &[NodeId]) {
        self.unanswered.remove(path);
    }
    pub(super) fn paused(&self, path: &[NodeId], now: Instant) -> bool {
        self.unanswered
            .get(path)
            .is_some_and(|paused| paused.until > now)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::node;
    use super::*;

    /// Each unanswered handshake after the previous pause ended doubles the
    /// pause; an answer resets it. At capacity a new path displaces the one
    /// whose pause ended longest ago, never one still paused.
    #[test]
    fn path_pauses_accumulate_across_their_expiry() {
        let base = std::time::Duration::from_secs(5);
        let path = vec![node(1), node(2)];
        let mut first = FirstPackets::default();
        let mut now = Instant::now();
        let mut pauses = Vec::new();
        for _ in 0..5 {
            first.unanswered(&path, None, now, base);
            let until = first.unanswered[&path].until;
            pauses.push((until - now).as_secs());
            assert!(first.paused(&path, now));
            now = until;
            assert!(!first.paused(&path, now), "eligible once the pause ends");
        }
        assert_eq!(pauses, [5, 10, 20, 40, 60]);
        first.answered(&path);
        first.unanswered(&path, None, now, base);
        assert_eq!((first.unanswered[&path].until - now).as_secs(), 5);

        let mut full = FirstPackets::default();
        let start = Instant::now();
        for i in 0..MAX_UNANSWERED {
            let at = start + std::time::Duration::from_millis(i as u64);
            full.unanswered(&[node(1), node(3), node(i as u8)], None, at, base);
        }
        let later = start + base + std::time::Duration::from_millis(1);
        full.unanswered(&[node(9), node(9)], None, later, base);
        assert_eq!(full.unanswered.len(), MAX_UNANSWERED);
        assert!(
            !full
                .unanswered
                .contains_key(&vec![node(1), node(3), node(0)])
        );
        assert!(full.unanswered.contains_key(&vec![node(9), node(9)]));
        // Nothing whose pause ended: a new path is not recorded.
        let mut paused = FirstPackets::default();
        for i in 0..MAX_UNANSWERED {
            paused.unanswered(&[node(2), node(i as u8)], None, start, base);
        }
        paused.unanswered(&[node(8), node(8)], None, start, base);
        assert!(!paused.unanswered.contains_key(&vec![node(8), node(8)]));
    }
}
