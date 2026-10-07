use super::*;
use routing::PathError;
use std::{
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
};

pub(super) struct Graph<'a> {
    input: Topology<'a>,
    pub candidates: RefCell<BTreeMap<NodeId, BTreeMap<String, DialCandidate>>>,
}
impl<'a> Graph<'a> {
    pub fn new(input: Topology<'a>) -> Result<Self, CompileError> {
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
        via: Option<&[NodeId]>,
        limit: usize,
    ) -> Result<Vec<LogicalPath>, PathError> {
        let mut routes = routing::Routes::new(self.input);
        let required = match via {
            Some(via) => routes.through(source, target, via)?,
            None => routes.segment(source, target)?,
        };
        if required.iter().any(|id| !self.available(*id)) {
            return Ok(vec![]);
        }
        let mut queue = VecDeque::from([(vec![source], required)]);
        let mut found = Vec::new();
        let mut truncated = false;
        'search: while let Some((path, required)) = queue.pop_front() {
            if path.len() >= MAX_HOPS {
                continue;
            }
            let a = *path.last().ok_or(CompileError::InvalidInput)?;
            // Required waypoint first keeps the common direct case bounded.
            let goal = required[0];
            let candidates = std::iter::once(goal)
                .chain(self.input.inventory.keys().copied().filter(|n| *n != goal));
            for b in candidates {
                if routes.spend().is_err() {
                    break 'search;
                }
                if path.contains(&b)
                    || required[1..].contains(&b)
                    || !self.available(b)
                    || !topology_allows(self.input.configuration, self.input.subjects, a, b)
                {
                    continue;
                }
                // A physically possible edge cannot bypass its own routing
                // requirement, even when introduced by automatic graph search.
                match routes.segment(a, b) {
                    Ok(segment) if segment == [b] => {}
                    Ok(_) | Err(PathError::Cycle) => continue,
                    Err(PathError::Limit) if routes.exhausted() => break 'search,
                    Err(PathError::Limit) => continue,
                    Err(error) => return Err(error),
                }
                let ab = transports::candidates(&self.input, a, b)?;
                let ba = transports::candidates(&self.input, b, a)?;
                if ab.is_empty() && ba.is_empty() {
                    continue;
                }
                let mut extended = path.clone();
                extended.push(b);
                let mut remaining = if b == goal {
                    Vec::new()
                } else {
                    match routes.segment(b, goal) {
                        Ok(segment) => segment,
                        Err(PathError::Cycle) => continue,
                        Err(PathError::Limit) if routes.exhausted() => break 'search,
                        Err(PathError::Limit) => continue,
                        Err(error) => return Err(error),
                    }
                };
                remaining.extend_from_slice(&required[1..]);
                if extended.len() + remaining.len() > MAX_HOPS
                    || remaining
                        .iter()
                        .any(|id| extended.contains(id) || !self.available(*id))
                    || remaining.iter().collect::<BTreeSet<_>>().len() != remaining.len()
                {
                    continue;
                }
                if remaining.is_empty() {
                    found.push(self.materialize(extended)?);
                    if found.len() == limit {
                        return Ok(found);
                    }
                } else if queue.len() < 2048 {
                    queue.push_back((extended, remaining));
                } else {
                    truncated = true;
                }
            }
        }
        if found.is_empty() && (truncated || routes.exhausted()) {
            Err(PathError::Limit)
        } else {
            Ok(found)
        }
    }
    fn available(&self, node: NodeId) -> bool {
        self.input.inventory.contains_key(&node)
            && self
                .input
                .configuration
                .nodes
                .get(&node)
                .is_some_and(|n| n.enabled)
    }
    fn materialize(&self, nodes: Vec<NodeId>) -> Result<LogicalPath, CompileError> {
        let mut keys: BTreeMap<NodeId, Vec<String>> = BTreeMap::new();
        for pair in nodes.windows(2) {
            for (a, b) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                for c in transports::candidates(&self.input, a, b)? {
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
