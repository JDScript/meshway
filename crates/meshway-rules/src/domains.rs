//! Bounds-checked reader for the `fst` 0.4 (format version 3) map that holds
//! the reversed domain names.
//!
//! The `fst` crate only builds this section. Its reader trusts its input and
//! may panic on malformed bytes, so loading untrusted bytes uses this reader:
//! every access is checked and returns `None` instead of panicking, and
//! [`validate`] proves at load time that every reachable node decodes, that
//! transitions only point to lower addresses (so walks terminate), that
//! inputs are sorted name bytes and that every key's value is exactly one
//! kind bit. After a successful [`validate`] no query can hit a decode
//! failure. The node encoding mirrors `fst::raw::node`.

use crate::{
    format::{EXACT, SUBDOMAIN, SUFFIX},
    name::name_byte,
};

const VERSION: u64 = 3;
const EMPTY: usize = 0;
/// Nodes with more transitions carry a 256-byte input index.
const INDEX_THRESHOLD: usize = 32;
/// `fst::raw::common_inputs::COMMON_INPUTS_INV[..63]`: one-transition nodes
/// store an index into this table instead of their input byte.
const COMMON_INPUTS: &[u8; 63] = b"te/oasripcnw.hlm-du012g=:bf3y5&_4v9678k%?xCDASFIBEjPTzRNM+LOqHG";

#[derive(Clone, Copy)]
pub(crate) struct Fst<'a> {
    data: &'a [u8],
    root: usize,
    len: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Node {
    /// Lowest byte of the node; transitions point strictly below it.
    end: usize,
    pub(crate) is_final: bool,
    pub(crate) final_output: u64,
    pub(crate) ntrans: usize,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Empty,
    Next {
        input: u8,
    },
    One {
        input: u8,
        output: u64,
        target: usize,
    },
    Any {
        tsize: usize,
        osize: usize,
        index: bool,
        /// One past the highest input byte (`input(i)` is at `inputs_end - i - 1`).
        inputs_end: usize,
        /// Lowest input byte; transition addresses are packed below it.
        inputs_start: usize,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct Transition {
    pub(crate) input: u8,
    pub(crate) output: u64,
    pub(crate) target: usize,
}

impl<'a> Fst<'a> {
    /// Header checks of `fst::raw::Fst::new` with checked arithmetic.
    pub(crate) fn parse(data: &'a [u8]) -> Option<Self> {
        let n = data.len();
        if n < 36 || crate::format::u64_at(data, 0)? != VERSION {
            return None;
        }
        // Type 0 is what `MapBuilder` writes.
        if crate::format::u64_at(data, 8)? != 0 {
            return None;
        }
        let root = usize::try_from(crate::format::u64_at(data, n - 12)?).ok()?;
        let len = crate::format::u64_at(data, n - 20)?;
        let valid = if root == EMPTY {
            n == 36
        } else {
            root.checked_add(21) == Some(n)
        };
        valid.then_some(Self { data, root, len })
    }

    pub(crate) fn len(&self) -> u64 {
        self.len
    }
    pub(crate) fn root(&self) -> Option<Node> {
        self.node(self.root)
    }

    fn byte(&self, at: usize) -> Option<u8> {
        self.data.get(at).copied()
    }
    fn uint(&self, at: usize, size: usize) -> Option<u64> {
        if !(1..=8).contains(&size) {
            return None;
        }
        let bytes = self.data.get(at..at.checked_add(size)?)?;
        Some(
            bytes
                .iter()
                .rev()
                .fold(0u64, |n, b| (n << 8) | u64::from(*b)),
        )
    }
    /// One-transition nodes: a common-input index in the state byte or the
    /// input byte just below it.
    fn single_input(&self, state: u8, addr: usize) -> Option<(u8, usize)> {
        match state & 0b0011_1111 {
            0 => Some((self.byte(addr.checked_sub(1)?)?, 1)),
            idx => Some((*COMMON_INPUTS.get(usize::from(idx) - 1)?, 0)),
        }
    }

    pub(crate) fn node(&self, addr: usize) -> Option<Node> {
        if addr == EMPTY {
            return Some(Node {
                end: EMPTY,
                is_final: true,
                final_output: 0,
                ntrans: 0,
                kind: Kind::Empty,
            });
        }
        let state = self.byte(addr)?;
        match state >> 6 {
            0b11 => {
                let (input, ilen) = self.single_input(state, addr)?;
                let end = addr - ilen;
                end.checked_sub(1)?;
                Some(Node {
                    end,
                    is_final: false,
                    final_output: 0,
                    ntrans: 1,
                    kind: Kind::Next { input },
                })
            }
            0b10 => {
                let (input, ilen) = self.single_input(state, addr)?;
                let sizes_at = addr.checked_sub(ilen + 1)?;
                let sizes = self.byte(sizes_at)?;
                let (tsize, osize) = (usize::from(sizes >> 4), usize::from(sizes & 15));
                let taddr_at = sizes_at.checked_sub(tsize)?;
                let delta = usize::try_from(self.uint(taddr_at, tsize)?).ok()?;
                let end = taddr_at.checked_sub(osize)?;
                let output = match osize {
                    0 => 0,
                    _ => self.uint(end, osize)?,
                };
                let target = match delta {
                    0 => EMPTY,
                    _ => end.checked_sub(delta)?,
                };
                Some(Node {
                    end,
                    is_final: false,
                    final_output: 0,
                    ntrans: 1,
                    kind: Kind::One {
                        input,
                        output,
                        target,
                    },
                })
            }
            _ => {
                let is_final = state & 0b0100_0000 != 0;
                let (ntrans, nlen) = match usize::from(state & 0b0011_1111) {
                    0 => match self.byte(addr.checked_sub(1)?)? {
                        1 => (256, 1),
                        n => (usize::from(n), 1),
                    },
                    n => (n, 0),
                };
                let sizes_at = addr.checked_sub(nlen + 1)?;
                let sizes = self.byte(sizes_at)?;
                let (tsize, osize) = (usize::from(sizes >> 4), usize::from(sizes & 15));
                if tsize > 8 || osize > 8 || (ntrans > 0 && tsize == 0) {
                    return None;
                }
                let index = ntrans > INDEX_THRESHOLD;
                let inputs_end = sizes_at.checked_sub(if index { 256 } else { 0 })?;
                let inputs_start = inputs_end.checked_sub(ntrans)?;
                let outputs_end = inputs_start.checked_sub(ntrans * tsize)?;
                let outputs_start = outputs_end.checked_sub(ntrans * osize)?;
                let (final_output, end) = match (is_final, osize) {
                    (false, _) => (0, outputs_start),
                    (true, 0) => (0, outputs_start),
                    (true, _) => {
                        let at = outputs_start.checked_sub(osize)?;
                        (self.uint(at, osize)?, at)
                    }
                };
                Some(Node {
                    end,
                    is_final,
                    final_output,
                    ntrans,
                    kind: Kind::Any {
                        tsize,
                        osize,
                        index,
                        inputs_end,
                        inputs_start,
                    },
                })
            }
        }
    }

    /// The `i`th transition in ascending input order.
    pub(crate) fn transition(&self, node: &Node, i: usize) -> Option<Transition> {
        match node.kind {
            Kind::Empty => None,
            Kind::Next { input } => (i == 0).then(|| Transition {
                input,
                output: 0,
                target: node.end - 1,
            }),
            Kind::One {
                input,
                output,
                target,
            } => (i == 0).then_some(Transition {
                input,
                output,
                target,
            }),
            Kind::Any {
                tsize,
                osize,
                inputs_end,
                inputs_start,
                ..
            } => {
                if i >= node.ntrans {
                    return None;
                }
                let input = self.byte(inputs_end.checked_sub(i + 1)?)?;
                let taddr_at = inputs_start.checked_sub((i + 1) * tsize)?;
                let delta = usize::try_from(self.uint(taddr_at, tsize)?).ok()?;
                let outputs_end = inputs_start.checked_sub(node.ntrans * tsize)?;
                let output = match osize {
                    0 => 0,
                    _ => self.uint(outputs_end.checked_sub((i + 1) * osize)?, osize)?,
                };
                let target = match delta {
                    0 => EMPTY,
                    _ => node.end.checked_sub(delta)?,
                };
                Some(Transition {
                    input,
                    output,
                    target,
                })
            }
        }
    }

    /// Transition for `input`, as `fst::raw::Node::find_input`.
    pub(crate) fn find(&self, node: &Node, input: u8) -> Option<Transition> {
        let i = match node.kind {
            Kind::Empty => return None,
            Kind::Next { input: own } | Kind::One { input: own, .. } => {
                if own != input {
                    return None;
                }
                0
            }
            Kind::Any {
                index: true,
                inputs_end,
                ..
            } => usize::from(self.byte(inputs_end.checked_add(usize::from(input))?)?),
            Kind::Any {
                inputs_end,
                inputs_start,
                ..
            } => {
                let inputs = self.data.get(inputs_start..inputs_end)?;
                let position = inputs.iter().position(|b| *b == input)?;
                node.ntrans - 1 - position
            }
        };
        self.transition(node, i)
    }
}

/// Load-time structural proof; see the module comment. `Some(())` iff valid.
///
/// The builder writes nodes back to back after the 16-byte header and the
/// root last, so the section must tile exactly: walking down from the root,
/// each node ends where the next lower one's state byte is. The walk visits
/// every node once in descending address order, which is a topological order
/// (transitions point strictly down), so one forward pass propagates output
/// sums, path counts and depths from the root. Transition targets must be tile
/// addresses, found in O(1) through a bitmap with rank counts.
pub(crate) fn validate(fst: &Fst<'_>) -> Option<()> {
    const HEADER: usize = 16;
    if fst.root == EMPTY {
        // Only the empty name, which is not a name.
        return None;
    }
    let tiles = || {
        let mut next = Some(fst.root);
        std::iter::from_fn(move || {
            let addr = next?;
            let node = fst.node(addr);
            next = node.and_then(|n| (n.end > HEADER).then(|| n.end - 1));
            Some(node.map(|n| (addr, n)))
        })
    };
    let mut bits = vec![0u64; fst.data.len().div_ceil(64)];
    let mut n = 0usize;
    let mut last_end = usize::MAX;
    for tile in tiles() {
        let (addr, node) = tile?;
        bits[addr / 64] |= 1 << (addr % 64);
        n += 1;
        last_end = node.end;
    }
    if last_end != HEADER {
        return None;
    }
    // rank[w] = tiles below word w.
    let mut rank = Vec::with_capacity(bits.len());
    let mut below = 0u32;
    for w in &bits {
        rank.push(below);
        below += w.count_ones();
    }
    // Index in walk (descending) order.
    let index_of = |addr: usize| -> Option<usize> {
        let word = *bits.get(addr / 64)?;
        if word & (1 << (addr % 64)) == 0 {
            return None;
        }
        let ascending =
            rank[addr / 64] as usize + (word & ((1 << (addr % 64)) - 1)).count_ones() as usize;
        Some(n - 1 - ascending)
    };
    let shift = |sums: u8, by: u64| -> Option<u8> {
        if sums == 0 {
            return Some(0);
        }
        let shifted = u16::from(sums).checked_shl(u32::try_from(by).ok()?)?;
        (shifted < 1 << 5).then_some(shifted as u8)
    };
    let allowed = (1u8 << EXACT) | (1 << SUFFIX) | (1 << SUBDOMAIN);
    let valid_final =
        |sums: u8, output: u64| shift(sums, output).is_some_and(|v| v != 0 && v & !allowed == 0);
    // Per node: bit `s` of `sums` set when some path from the root arrives
    // with output sum `s` (0..=4; larger sums cannot end in a valid value),
    // the number of such paths, and the longest one.
    let mut sums = vec![0u8; n];
    let mut paths = vec![0u64; n];
    let mut depth = vec![0u8; n];
    sums[0] = 1;
    paths[0] = 1;
    let mut keys = 0u64;
    for (i, tile) in tiles().enumerate() {
        let (_, node) = tile?;
        if node.is_final {
            if i == 0 || !valid_final(sums[i], node.final_output) {
                return None;
            }
            keys = keys.saturating_add(paths[i]);
        }
        let mut previous: Option<u8> = None;
        for t in 0..node.ntrans {
            let t = fst.transition(&node, t)?;
            if !name_byte(t.input) || previous.is_some_and(|p| p >= t.input) {
                return None;
            }
            previous = Some(t.input);
            // Sorted unique inputs make the linear search agree with
            // iteration; an input index must be checked entry by entry.
            if matches!(node.kind, Kind::Any { index: true, .. })
                && fst.find(&node, t.input).map(|f| f.target) != Some(t.target)
            {
                return None;
            }
            let s = shift(sums[i], t.output)?;
            let d = depth[i].checked_add(1).filter(|d| *d <= 253)?;
            if t.target == EMPTY {
                if !valid_final(s, 0) {
                    return None;
                }
                keys = keys.saturating_add(paths[i]);
                continue;
            }
            let c = index_of(t.target)?;
            sums[c] |= s;
            paths[c] = paths[c].saturating_add(paths[i]);
            depth[c] = depth[c].max(d);
        }
        if let Kind::Any {
            index: true,
            inputs_end,
            ..
        } = node.kind
        {
            // The index may not name a transition for any other byte.
            let table = fst.data.get(inputs_end..inputs_end.checked_add(256)?)?;
            if table
                .iter()
                .filter(|i| usize::from(**i) < node.ntrans)
                .count()
                != node.ntrans
            {
                return None;
            }
        }
    }
    (keys == fst.len).then_some(())
}

/// Depth-first walk over every key in byte order of the reversed name.
/// Only used on validated sections, where it is bounded by the key count.
pub(crate) struct Keys<'a> {
    fst: Fst<'a>,
    /// (node, next transition, output sum before the node)
    stack: Vec<(Node, usize, u64)>,
    key: Vec<u8>,
    emitted_root: bool,
}

impl<'a> Keys<'a> {
    pub(crate) fn new(fst: Fst<'a>) -> Self {
        let stack = fst.root().map(|n| (n, 0, 0)).into_iter().collect();
        Self {
            fst,
            stack,
            key: vec![],
            emitted_root: false,
        }
    }
}

impl Iterator for Keys<'_> {
    /// (reversed key, value)
    type Item = (Vec<u8>, u64);
    fn next(&mut self) -> Option<Self::Item> {
        if !self.emitted_root {
            self.emitted_root = true;
            let (root, _, _) = self.stack.first()?;
            if root.is_final {
                return Some((vec![], root.final_output));
            }
        }
        loop {
            let (node, i, sum) = self.stack.last_mut()?;
            if *i >= node.ntrans {
                self.stack.pop();
                self.key.pop();
                continue;
            }
            let t = self.fst.transition(node, *i)?;
            *i += 1;
            let sum = sum.wrapping_add(t.output);
            let child = self.fst.node(t.target)?;
            self.key.push(t.input);
            self.stack.push((child, 0, sum));
            if child.is_final {
                return Some((self.key.clone(), sum.wrapping_add(child.final_output)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn build(keys: &std::collections::BTreeMap<Vec<u8>, u64>) -> Vec<u8> {
        let mut b = fst::MapBuilder::memory();
        for (k, v) in keys {
            b.insert(k, *v).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn lookup(fst: &Fst<'_>, key: &[u8]) -> Option<u64> {
        let mut node = fst.root()?;
        let mut sum = 0u64;
        for b in key {
            let t = fst.find(&node, *b)?;
            sum += t.output;
            node = fst.node(t.target)?;
        }
        node.is_final.then_some(sum + node.final_output)
    }

    fn keys() -> impl Strategy<Value = std::collections::BTreeMap<Vec<u8>, u64>> {
        // Small alphabets force shared suffixes; the wide one forces
        // index nodes (> 32 transitions).
        let narrow = prop::collection::vec(prop::sample::select(b"ab.-".to_vec()), 1..12);
        let wide = prop::collection::vec(
            prop::sample::select(b"abcdefghijklmnopqrstuvwxyz0123456789.-".to_vec()),
            1..4,
        );
        let key = prop_oneof![narrow, wide];
        prop::collection::btree_map(key, prop::sample::select(vec![1u64, 2, 4]), 0..300)
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]
        #[test]
        fn reader_agrees_with_fst_crate(keys in keys(), probes in prop::collection::vec(prop::collection::vec(prop::sample::select(b"abc.-z".to_vec()), 0..8), 0..50)) {
            let bytes = build(&keys);
            let ours = Fst::parse(&bytes).unwrap();
            prop_assert!(validate(&ours).is_some());
            let theirs = fst::Map::new(bytes.clone()).unwrap();
            prop_assert_eq!(ours.len(), theirs.len() as u64);
            for (k, v) in &keys {
                prop_assert_eq!(lookup(&ours, k), Some(*v));
            }
            for p in &probes {
                prop_assert_eq!(lookup(&ours, p), theirs.get(p));
            }
            let walked: Vec<(Vec<u8>, u64)> = Keys::new(ours).collect();
            let expected: Vec<(Vec<u8>, u64)> = keys.into_iter().collect();
            prop_assert_eq!(walked, expected);
        }
    }

    #[test]
    fn common_inputs_match_the_builder() {
        // Every single-byte key in its own map exercises the common-input
        // index of a one-transition root.
        for b in 0u8..=255 {
            let mut keys = std::collections::BTreeMap::new();
            keys.insert(vec![b, b'x'], 2u64);
            let bytes = build(&keys);
            let ours = Fst::parse(&bytes).unwrap();
            assert_eq!(lookup(&ours, &[b, b'x']), Some(2), "byte {b}");
        }
    }

    #[test]
    fn rejects_values_outside_the_kind_bits_and_non_name_bytes() {
        for (key, value) in [
            (b"a".to_vec(), 3u64),
            (b"a".to_vec(), 8),
            (b"A".to_vec(), 1),
        ] {
            let bytes = build(&[(key, value)].into_iter().collect());
            assert!(validate(&Fst::parse(&bytes).unwrap()).is_none());
        }
        let empty = build(&Default::default());
        assert!(validate(&Fst::parse(&empty).unwrap()).is_some());
    }
}
