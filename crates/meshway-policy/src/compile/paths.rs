use super::*;
use routing::PathError;
use std::{
    cell::RefCell,
    collections::{BTreeSet, VecDeque},
};

pub(super) struct Graph<'a> {
    input: Topology<'a>,
    declarations_only: bool,
    pub transport_failures: RefCell<BTreeMap<(NodeId, NodeId), usize>>,
    pub candidates: RefCell<BTreeMap<NodeId, BTreeMap<String, DialCandidate>>>,
    /// Pure functions of the fixed input, memoized across searches of one compilation.
    topology: RefCell<BTreeMap<(NodeId, NodeId), bool>>,
    carriers: RefCell<BTreeMap<(NodeId, NodeId), Vec<DialCandidate>>>,
    /// Netmap projections carry redacted declarations: carriers are derived
    /// from them as listener keys only, never as dial material.
    views: Option<&'a ConfigurationView>,
    offers: RefCell<BTreeMap<(NodeId, NodeId), Vec<String>>>,
}
impl<'a> Graph<'a> {
    pub fn new(input: Topology<'a>) -> Result<Self, CompileError> {
        if input.inventory.len() > 10_000 || input.configuration.nodes.len() > 10_000 {
            return Err(CompileError::Budget);
        }
        Ok(Self {
            input,
            declarations_only: false,
            transport_failures: RefCell::new(BTreeMap::new()),
            candidates: RefCell::new(BTreeMap::new()),
            topology: RefCell::new(BTreeMap::new()),
            carriers: RefCell::new(BTreeMap::new()),
            views: None,
            offers: RefCell::new(BTreeMap::new()),
        })
    }
    /// Node-side search over a netmap projection, using the same route search.
    pub fn projected(
        input: Topology<'a>,
        views: &'a ConfigurationView,
    ) -> Result<Self, CompileError> {
        let mut graph = Self::new(input)?;
        graph.views = Some(views);
        Ok(graph)
    }
    /// Preview shares route search, but neither requires trusted inventory nor
    /// manufactures credentials/candidates. Its paths are never published.
    pub fn declarations(input: Topology<'a>) -> Result<Self, CompileError> {
        let mut graph = Self::new(input)?;
        graph.declarations_only = true;
        Ok(graph)
    }
    pub fn paths(
        &self,
        source: NodeId,
        target: NodeId,
        via: Option<&[NodeId]>,
        limit: usize,
    ) -> Result<Vec<LogicalPath>, PathError> {
        self.transport_failures.borrow_mut().clear();
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
            let candidates = std::iter::once(goal).chain(
                self.input
                    .configuration
                    .nodes
                    .keys()
                    .copied()
                    .filter(|n| *n != goal),
            );
            for b in candidates {
                // Budget only edges the topology allows: Nodes outside the
                // searched component (absent from a projection) cost nothing.
                if path.contains(&b)
                    || required[1..].contains(&b)
                    || !self.available(b)
                    || !self.topology_allows(a, b)
                {
                    continue;
                }
                if routes.spend().is_err() {
                    break 'search;
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
                if !self.has_transport(a, b)? {
                    if let Some(Some(index)) = crate::topology_decision(
                        self.input.configuration,
                        self.input.subjects,
                        a,
                        b,
                    ) && matches!(
                        self.input.configuration.topology[index].transports,
                        Input::Value(_)
                    ) {
                        self.transport_failures
                            .borrow_mut()
                            .insert((a.min(b), a.max(b)), index);
                    }
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
        } else if found.is_empty()
            && let Some((&(a, b), &rule)) = self.transport_failures.borrow().first_key_value()
        {
            Err(CompileError::EdgeTransportUnavailable { rule, a, b }.into())
        } else {
            Ok(found)
        }
    }
    fn has_transport(&self, a: NodeId, b: NodeId) -> Result<bool, CompileError> {
        if self.declarations_only {
            return Ok(
                transports::declared(self.input.configuration, self.input.subjects, a, b)
                    || transports::declared(self.input.configuration, self.input.subjects, b, a),
            );
        }
        Ok(self.has_carrier(a, b)? || self.has_carrier(b, a)?)
    }
    pub fn topology_allows(&self, a: NodeId, b: NodeId) -> bool {
        *self
            .topology
            .borrow_mut()
            .entry((a, b))
            .or_insert_with(|| topology_allows(self.input.configuration, self.input.subjects, a, b))
    }
    fn has_carrier(&self, a: NodeId, b: NodeId) -> Result<bool, CompileError> {
        if self.views.is_some() {
            return Ok(!self.listener_keys(a, b)?.is_empty());
        }
        if let Some(cached) = self.carriers.borrow().get(&(a, b)) {
            return Ok(!cached.is_empty());
        }
        Ok(!self.carriers(a, b)?.is_empty())
    }
    /// Listener keys of `b` that `a` may dial, in carrier order.
    pub fn listener_keys(&self, a: NodeId, b: NodeId) -> Result<Vec<String>, CompileError> {
        let Some(views) = self.views else {
            return Ok(self
                .carriers(a, b)?
                .into_iter()
                .map(|c| c.listener_key)
                .collect());
        };
        if let Some(cached) = self.offers.borrow().get(&(a, b)) {
            return Ok(cached.clone());
        }
        let found = transports::offers(&self.input, views, a, b)?;
        self.offers.borrow_mut().insert((a, b), found.clone());
        Ok(found)
    }
    /// Dial candidates from `a` to `b`; errors are not memoized and abort the caller.
    pub fn carriers(&self, a: NodeId, b: NodeId) -> Result<Vec<DialCandidate>, CompileError> {
        if let Some(cached) = self.carriers.borrow().get(&(a, b)) {
            return Ok(cached.clone());
        }
        let found = transports::candidates(&self.input, a, b)?;
        self.carriers.borrow_mut().insert((a, b), found.clone());
        Ok(found)
    }
    fn available(&self, node: NodeId) -> bool {
        (self.declarations_only || self.input.inventory.contains_key(&node))
            && self
                .input
                .configuration
                .nodes
                .get(&node)
                .is_some_and(|n| n.enabled)
    }
    fn materialize(&self, nodes: Vec<NodeId>) -> Result<LogicalPath, CompileError> {
        let mut keys: BTreeMap<NodeId, Vec<String>> = BTreeMap::new();
        if self.declarations_only {
            return Ok(LogicalPath {
                nodes,
                dial_candidates: keys,
            });
        }
        for pair in nodes.windows(2) {
            for (a, b) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                if self.views.is_some() {
                    for listener in self.listener_keys(a, b)? {
                        keys.entry(a).or_default().push(format!("{b}/{listener}"));
                    }
                    continue;
                }
                for c in self.carriers(a, b)? {
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
