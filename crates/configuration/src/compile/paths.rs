use super::*;
use std::{
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
};

pub(super) struct Graph<'a, 'b> {
    input: &'a CompilationInput<'b>,
    pub candidates: RefCell<BTreeMap<NodeId, BTreeMap<String, DialCandidate>>>,
}
impl<'a, 'b> Graph<'a, 'b> {
    pub fn new(input: &'a CompilationInput<'b>) -> Result<Self, CompileError> {
        if input.inventory.len() > 10_000 {
            return Err(CompileError::Budget);
        }
        Ok(Self {
            input,
            candidates: RefCell::new(BTreeMap::new()),
        })
    }
    pub fn paths(
        &self,
        source: NodeId,
        target: NodeId,
        via: &[NodeId],
        limit: usize,
    ) -> Result<Vec<LogicalPath>, CompileError> {
        let mut required = via.to_vec();
        required.push(target);
        if required.len() >= MAX_HOPS
            || required.contains(&source)
            || required.iter().collect::<BTreeSet<_>>().len() != required.len()
        {
            return Ok(vec![]);
        }
        let mut queue = VecDeque::from([(vec![source], 0usize)]);
        let mut found = Vec::new();
        let mut work = 0usize;
        while let Some((path, next_required)) = queue.pop_front() {
            if path.len() >= MAX_HOPS {
                continue;
            }
            let a = *path.last().ok_or(CompileError::InvalidInput)?;
            // Required waypoint first keeps the common direct case bounded.
            let candidates = std::iter::once(required[next_required]).chain(
                self.input
                    .inventory
                    .keys()
                    .copied()
                    .filter(|n| *n != required[next_required]),
            );
            for b in candidates {
                work += 1;
                if work > 16_384 {
                    return if found.is_empty() {
                        Err(CompileError::Budget)
                    } else {
                        Ok(found)
                    };
                }
                if path.contains(&b)
                    || required[next_required + 1..].contains(&b)
                    || !topology_allows(self.input.configuration, a, b)
                {
                    continue;
                }
                let ab = transports::candidates(self.input, a, b)?;
                let ba = transports::candidates(self.input, b, a)?;
                if ab.is_empty() && ba.is_empty() {
                    continue;
                }
                let mut extended = path.clone();
                extended.push(b);
                let next = next_required + usize::from(b == required[next_required]);
                if next == required.len() {
                    found.push(self.materialize(extended)?);
                    if found.len() == limit {
                        return Ok(found);
                    }
                } else if queue.len() < 2048 {
                    queue.push_back((extended, next));
                }
            }
        }
        Ok(found)
    }
    fn materialize(&self, nodes: Vec<NodeId>) -> Result<LogicalPath, CompileError> {
        let mut keys: BTreeMap<NodeId, Vec<String>> = BTreeMap::new();
        for pair in nodes.windows(2) {
            for (a, b) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                for c in transports::candidates(self.input, a, b)? {
                    let key = format!("{b}/{}", c.listener_key);
                    keys.entry(a).or_default().push(key.clone());
                    self.candidates
                        .borrow_mut()
                        .entry(a)
                        .or_default()
                        .insert(key, c);
                }
            }
        }
        Ok(LogicalPath {
            nodes,
            dial_candidates: keys,
        })
    }
}
