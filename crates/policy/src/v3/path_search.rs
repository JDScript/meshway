//! Shared bounded, deterministic primary/standby path search.
use super::*;
use std::{
    cmp::Reverse,
    collections::{BinaryHeap, VecDeque},
};

pub(super) fn find_paths(
    graph: &[Vec<u16>],
    bridges: &BTreeSet<(u16, u16)>,
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    count: usize,
) -> Result<Vec<Vec<u16>>, String> {
    let primary = search(graph, nodes, src, dst, via, None)?;
    let Some(primary) = primary else {
        return Err("Unreachable mandatory path within eight-node budget".into());
    };
    let mut paths = vec![primary.clone()];
    if count > 1 {
        let mut alternatives = Vec::new();
        for e in primary.windows(2) {
            if bridges.contains(&(e[0].min(e[1]), e[0].max(e[1]))) {
                continue;
            }
            if let Some(p) = search(graph, nodes, src, dst, via, Some((e[0], e[1])))? {
                alternatives.push(p);
            }
        }
        alternatives.sort_by(|a, b| a.len().cmp(&b.len()).then(a.cmp(b)));
        if let Some(p) = alternatives.into_iter().next() {
            paths.push(p);
        }
    }
    Ok(paths)
}
fn search(
    graph: &[Vec<u16>],
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    banned: Option<(u16, u16)>,
) -> Result<Option<Vec<u16>>, String> {
    if src == dst {
        return Ok(via.is_empty().then_some(vec![src as u16]));
    }
    if via.is_empty() {
        let edge_allowed = |a: usize, b: usize| {
            !banned.is_some_and(|(x, y)| {
                (x == a as u16 && y == b as u16) || (y == a as u16 && x == b as u16)
            })
        };
        if graph[src].binary_search(&(dst as u16)).is_ok() && edge_allowed(src, dst) {
            return Ok(Some(vec![src as u16, dst as u16]));
        }
        for &mid in &graph[src] {
            let m = usize::from(mid);
            if edge_allowed(src, m)
                && edge_allowed(m, dst)
                && graph[m].binary_search(&(dst as u16)).is_ok()
            {
                return Ok(Some(vec![src as u16, mid, dst as u16]));
            }
        }
        let mut previous = vec![usize::MAX; nodes.len()];
        previous[src] = src;
        let mut queue = VecDeque::from([src]);
        while let Some(a) = queue.pop_front() {
            for &b in &graph[a] {
                let b = usize::from(b);
                if banned.is_some_and(|(x, y)| {
                    (x == a as u16 && y == b as u16) || (y == a as u16 && x == b as u16)
                }) || previous[b] != usize::MAX
                {
                    continue;
                }
                previous[b] = a;
                if b == dst {
                    let mut p = vec![dst as u16];
                    let mut i = dst;
                    while i != src {
                        i = previous[i];
                        p.push(i as u16);
                    }
                    p.reverse();
                    return Ok((p.len() <= MAX_PATH_NODES).then_some(p));
                }
                queue.push_back(b);
            }
        }
        return Ok(None);
    }
    // Mandatory ordered hops plus simple paths is constrained search; never prune
    // by (node,stage) alone, which would lose distinct visited-node histories.
    let remaining = remaining_distances(graph, nodes, src, dst, via, banned);
    if remaining[0][src] == usize::MAX {
        return Ok(None);
    }
    let mut queue = BinaryHeap::from([Reverse((remaining[0][src] + 1, vec![src as u16], 0usize))]);
    let mut states = 0;
    while let Some(Reverse((_, path, stage))) = queue.pop() {
        states += 1;
        if states > MAX_SEARCH_STATES {
            return Err("Search budget exhausted; no bypass installed".into());
        }
        let a = usize::from(*path.last().expect("nonempty"));
        if a == dst {
            if stage == via.len() {
                return Ok(Some(path));
            }
            continue;
        }
        if path.len() == MAX_PATH_NODES {
            continue;
        }
        for &b in &graph[a] {
            if path.contains(&b)
                || banned
                    .is_some_and(|(x, y)| (x == a as u16 && y == b) || (y == a as u16 && x == b))
            {
                continue;
            }
            let bi = usize::from(b);
            let advance = stage < via.len() && matches(&via[stage], nodes[bi]) && bi != dst;
            // A concrete later mandatory hop cannot be consumed early.
            if !advance
                && via[stage..]
                    .iter()
                    .any(|s| s.starts_with("node:") && matches(s, nodes[bi]))
            {
                continue;
            }
            let mut p = path.clone();
            p.push(b);
            let next_stage = stage + usize::from(advance);
            let lower_bound = p.len().saturating_add(remaining[next_stage][bi]);
            if lower_bound <= MAX_PATH_NODES {
                queue.push(Reverse((lower_bound, p, next_stage)));
            }
            if queue.len() > MAX_SEARCH_STATES {
                return Err("Search budget exhausted; no bypass installed".into());
            }
        }
    }
    Ok(None)
}

/// Relaxed ordered walk distances are an admissible A* heuristic for simple paths.
fn remaining_distances(
    graph: &[Vec<u16>],
    nodes: &[&Node],
    src: usize,
    dst: usize,
    via: &[String],
    banned: Option<(u16, u16)>,
) -> Vec<Vec<usize>> {
    let mut distances = vec![vec![usize::MAX; nodes.len()]; via.len() + 1];
    for stage in (0..=via.len()).rev() {
        let mut heap = BinaryHeap::new();
        if stage == via.len() {
            distances[stage][dst] = 0;
            heap.push(Reverse((0usize, dst)));
        } else {
            for (i, node) in nodes.iter().enumerate() {
                if i != src
                    && i != dst
                    && matches(&via[stage], node)
                    && distances[stage + 1][i] != usize::MAX
                {
                    let cost = distances[stage + 1][i];
                    distances[stage][i] = cost;
                    heap.push(Reverse((cost, i)));
                }
            }
        }
        while let Some(Reverse((cost, a))) = heap.pop() {
            if distances[stage][a] != cost {
                continue;
            }
            for &b in &graph[a] {
                if banned
                    .is_some_and(|(x, y)| (x == a as u16 && y == b) || (y == a as u16 && x == b))
                {
                    continue;
                }
                let b = usize::from(b);
                let next = cost + 1;
                if next < distances[stage][b] {
                    distances[stage][b] = next;
                    heap.push(Reverse((next, b)));
                }
            }
        }
    }
    distances
}
