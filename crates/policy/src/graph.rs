//! One deterministic adjacency graph shared by Mesh and external path planning.
use super::*;

pub(super) struct Graph<'a> {
    pub nodes: Vec<&'a Node>,
    pub dial: Vec<Vec<bool>>,
    pub edges: Vec<Vec<u16>>,
    pub bridges: BTreeSet<(u16, u16)>,
}
impl<'a> Graph<'a> {
    /// Inputs must have passed validation before indexing reachability facts.
    pub fn new(p: &Policy, facts: &'a Snapshot) -> Self {
        let mut nodes: Vec<_> = facts.nodes.iter().collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let n = nodes.len();
        let index: BTreeMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        let mut dial = vec![vec![false; n]; n];
        for d in &facts.reachable {
            let a = index[d.from.as_str()];
            let b = index[d.to.as_str()];
            if nodes[b].listener {
                dial[a][b] = true;
            }
        }
        let mut edges = vec![Vec::new(); n];
        for a in 0..n {
            for b in a + 1..n {
                if !dial[a][b] && !dial[b][a] {
                    continue;
                }
                if connection_allowed(&p.connections, nodes[a], nodes[b]) {
                    edges[a].push(b as u16);
                    edges[b].push(a as u16);
                }
            }
        }
        let bridges = graph_bridges(&edges);
        Self {
            nodes,
            dial,
            edges,
            bridges,
        }
    }
}

fn graph_bridges(graph: &[Vec<u16>]) -> BTreeSet<(u16, u16)> {
    struct Search<'a> {
        graph: &'a [Vec<u16>],
        time: usize,
        entered: Vec<usize>,
        low: Vec<usize>,
        bridges: BTreeSet<(u16, u16)>,
    }
    impl Search<'_> {
        fn visit(&mut self, a: usize, parent: usize) {
            self.time += 1;
            self.entered[a] = self.time;
            self.low[a] = self.time;
            for &b in &self.graph[a] {
                let b = usize::from(b);
                if b == parent {
                    continue;
                }
                if self.entered[b] == 0 {
                    self.visit(b, a);
                    self.low[a] = self.low[a].min(self.low[b]);
                    if self.low[b] > self.entered[a] {
                        self.bridges.insert((a.min(b) as u16, a.max(b) as u16));
                    }
                } else {
                    self.low[a] = self.low[a].min(self.entered[b]);
                }
            }
        }
    }
    let mut search = Search {
        graph,
        time: 0,
        entered: vec![0; graph.len()],
        low: vec![0; graph.len()],
        bridges: BTreeSet::new(),
    };
    for a in 0..graph.len() {
        if search.entered[a] == 0 {
            search.visit(a, usize::MAX);
        }
    }
    search.bridges
}
