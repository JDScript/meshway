//! Resolve ordered Mesh rules independently of business Access and physical I/O.
//! Each segment matches its own endpoints. Only a boundary occurrence of a
//! waypoint is already satisfied; interior repeats and recursive dependencies
//! are errors, never a reason to consult a later rule.
use super::*;
use std::collections::BTreeSet;

const MAX_WORK: usize = 16_384;

pub(super) enum PathError {
    Cycle,
    Limit,
    Compile(CompileError),
}

impl From<CompileError> for PathError {
    fn from(error: CompileError) -> Self {
        Self::Compile(error)
    }
}

pub(super) struct Routes<'a, 'b> {
    input: &'a CompilationInput<'b>,
    resolved: BTreeMap<(NodeId, NodeId), Vec<NodeId>>,
    active: BTreeSet<(NodeId, NodeId)>,
    work: usize,
}

impl<'a, 'b> Routes<'a, 'b> {
    pub fn new(input: &'a CompilationInput<'b>) -> Self {
        Self {
            input,
            resolved: BTreeMap::new(),
            active: BTreeSet::new(),
            work: 0,
        }
    }

    /// The whole search shares a budget: recursion, selector scans and physical
    /// candidates cannot each obtain a fresh unbounded allowance.
    pub fn spend(&mut self) -> Result<(), PathError> {
        self.work += 1;
        if self.work > MAX_WORK {
            Err(PathError::Limit)
        } else {
            Ok(())
        }
    }

    pub fn exhausted(&self) -> bool {
        self.work > MAX_WORK
    }

    /// Returns all required destinations, excluding source and including target.
    pub fn segment(&mut self, source: NodeId, target: NodeId) -> Result<Vec<NodeId>, PathError> {
        self.spend()?;
        if source == target {
            return Err(PathError::Cycle);
        }
        if let Some(path) = self.resolved.get(&(source, target)) {
            return Ok(path.clone());
        }
        if !self.active.insert((source, target)) {
            return Err(PathError::Cycle);
        }
        let result = self.resolve(source, target);
        self.active.remove(&(source, target));
        if let Ok(path) = &result {
            self.resolved.insert((source, target), path.clone());
        }
        result
    }

    fn resolve(&mut self, source: NodeId, target: NodeId) -> Result<Vec<NodeId>, PathError> {
        if self.active.len() >= MAX_HOPS {
            return Err(PathError::Limit);
        }
        let mut via = Vec::new();
        for rule in &self.input.configuration.routing.mesh {
            self.spend()?;
            if matching::selectors(
                self.input.configuration,
                self.input.subjects,
                &rule.src,
                source,
            ) && matching::selectors(
                self.input.configuration,
                self.input.subjects,
                &rule.dst,
                target,
            ) {
                if rule.via.len() > MAX_HOPS {
                    return Err(PathError::Limit);
                }
                via = rule
                    .via
                    .iter()
                    .map(|s| matching::node_selector(s))
                    .collect::<Result<_, _>>()?;
                break;
            }
        }
        let via = boundaries(source, target, &via)?;
        if via.is_empty() {
            return Ok(vec![target]);
        }
        self.through(source, target, via)
    }

    /// Egress waypoints are selected by the original business request. Resolving
    /// their internal Mesh segments must not redo egress or Access as a relay.
    pub fn through(
        &mut self,
        source: NodeId,
        target: NodeId,
        via: &[NodeId],
    ) -> Result<Vec<NodeId>, PathError> {
        let via = boundaries(source, target, via)?;
        let mut path = Vec::new();
        let mut previous = source;
        for next in via.iter().copied().chain(std::iter::once(target)) {
            let segment = self.segment(previous, next)?;
            for node in segment {
                if node == source || path.contains(&node) {
                    return Err(PathError::Cycle);
                }
                path.push(node);
                if path.len() >= MAX_HOPS {
                    return Err(PathError::Limit);
                }
            }
            previous = next;
        }
        Ok(path)
    }
}

fn boundaries(source: NodeId, target: NodeId, via: &[NodeId]) -> Result<&[NodeId], PathError> {
    // Check before trimming: [source, source] is not two satisfied boundaries.
    if via.iter().collect::<BTreeSet<_>>().len() != via.len() {
        return Err(PathError::Cycle);
    }
    let via = if via.first() == Some(&source) {
        &via[1..]
    } else {
        via
    };
    let via = if via.last() == Some(&target) {
        &via[..via.len() - 1]
    } else {
        via
    };
    if via.contains(&source) || via.contains(&target) {
        return Err(PathError::Cycle);
    }
    if via.len() + 2 > MAX_HOPS {
        return Err(PathError::Limit);
    }
    Ok(via)
}
