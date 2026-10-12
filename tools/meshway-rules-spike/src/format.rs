//! Prototype `MWRS` v1 artifact: builder, zero-deserialisation loader and
//! matcher. Layout (little-endian, sections 8-byte aligned):
//!
//! ```text
//! 0   magic "MWRULES\0"
//! 8   major u16, minor u16, flags u32
//! 16  section_count u32, reserved u32
//! 24  total_len u64
//! 32  exact u32, suffix u32, keyword u32, v4_ranges u32, v6_ranges u32, subdomain u32
//! 56  reserved (zero) to 64
//! 64  section table: count x { kind u32, flags u32, offset u64, len u64 }
//! ..  sections
//! ```

use crate::source::Parsed;
use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::net::IpAddr;
use std::ops::Range;
use std::sync::Arc;

pub const MAGIC: &[u8; 8] = b"MWRULES\0";
pub const MAJOR: u16 = 1;
const HEADER: usize = 64;
const SECTION: usize = 24;
const REQUIRED: u32 = 1;
const DOMAINS: u32 = 1;
const KEYWORDS: u32 = 2;
const IPV4: u32 = 3;
const IPV6: u32 = 4;
/// Optional alternative domain index measured against the FST.
const DOMAIN_HASH: u32 = 5;
const EXACT_BIT: u64 = 1;
const SUFFIX_BIT: u64 = 2;
const SUBDOMAIN_BIT: u64 = 4;

/// Set-equal inputs produce equal canonical sets and therefore equal bytes.
#[derive(Default)]
pub struct Canonical {
    pub exact: BTreeSet<String>,
    pub suffix: BTreeSet<String>,
    pub subdomain: BTreeSet<String>,
    pub keyword: BTreeSet<String>,
    pub v4: Vec<(u32, u32)>,
    pub v6: Vec<(u128, u128)>,
    /// Entries dropped because another entry of the same set already covers them.
    pub redundant: usize,
}

fn label_suffixes(name: &str) -> impl Iterator<Item = &str> {
    std::iter::once(name).chain(name.match_indices('.').map(move |(i, _)| &name[i + 1..]))
}

pub fn canonicalize(p: &Parsed) -> Canonical {
    let mut c = Canonical::default();
    // exact(n) + subdomain(n) is exactly suffix(n).
    let exact_in: HashSet<&str> = p.exact.iter().map(String::as_str).collect();
    let sub_in: HashSet<&str> = p.subdomain.iter().map(String::as_str).collect();
    let promoted: Vec<String> = sub_in
        .iter()
        .filter(|n| exact_in.contains(**n))
        .map(|n| (*n).to_owned())
        .collect();
    let all_suffix: Vec<String> = p.suffix.iter().chain(&promoted).cloned().collect();
    let suffixes: HashSet<&str> = all_suffix.iter().map(String::as_str).collect();
    let keywords: BTreeSet<&str> = p.keyword.iter().map(String::as_str).collect();
    // Keyword covered by a shorter keyword it contains.
    for k in &keywords {
        if keywords.iter().any(|o| o != k && k.contains(o)) {
            c.redundant += 1;
        } else {
            c.keyword.insert((*k).to_owned());
        }
    }
    let ac = (!c.keyword.is_empty()).then(|| AhoCorasick::new(&c.keyword).expect("keywords"));
    let by_keyword = |n: &str| ac.as_ref().is_some_and(|a| a.is_match(n));
    for s in &all_suffix {
        let covered = label_suffixes(s).skip(1).any(|x| suffixes.contains(x));
        if covered || by_keyword(s) || c.suffix.contains(s) {
            c.redundant += 1;
        } else {
            c.suffix.insert(s.clone());
        }
    }
    for e in &p.exact {
        let covered = label_suffixes(e).any(|x| suffixes.contains(x));
        if covered || by_keyword(e) || c.exact.contains(e) {
            c.redundant += 1;
        } else {
            c.exact.insert(e.clone());
        }
    }
    for d in &p.subdomain {
        let covered = label_suffixes(d).any(|x| suffixes.contains(x));
        if covered || by_keyword(d) || c.subdomain.contains(d) {
            c.redundant += 1;
        } else {
            c.subdomain.insert(d.clone());
        }
    }
    c.redundant -= promoted.len().min(c.redundant);
    let mut v4 = vec![];
    let mut v6 = vec![];
    for (addr, bits) in &p.cidr {
        match addr {
            IpAddr::V4(a) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(*bits)).unwrap_or(0);
                let start = u32::from(*a) & mask;
                v4.push((start, start | !mask));
            }
            IpAddr::V6(a) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(*bits)).unwrap_or(0);
                let start = u128::from(*a) & mask;
                v6.push((start, start | !mask));
            }
        }
    }
    c.v4 = merge4(v4);
    c.v6 = merge6(v6);
    c
}

macro_rules! merge_fn {
    ($name:ident, $t:ty) => {
        fn $name(mut ranges: Vec<($t, $t)>) -> Vec<($t, $t)> {
            ranges.sort();
            let mut out: Vec<($t, $t)> = vec![];
            for (s, e) in ranges {
                match out.last_mut() {
                    Some(last) if s <= last.1 || last.1.checked_add(1) == Some(s) => {
                        last.1 = last.1.max(e);
                    }
                    _ => out.push((s, e)),
                }
            }
            out
        }
    };
}
merge_fn!(merge4, u32);
merge_fn!(merge6, u128);

fn reversed(name: &str) -> Vec<u8> {
    name.bytes().rev().collect()
}

pub fn build(c: &Canonical) -> Vec<u8> {
    build_with(c, false)
}

pub fn build_with(c: &Canonical, hashed: bool) -> Vec<u8> {
    let mut keys: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
    for e in &c.exact {
        *keys.entry(reversed(e)).or_default() |= EXACT_BIT;
    }
    for s in &c.suffix {
        *keys.entry(reversed(s)).or_default() |= SUFFIX_BIT;
    }
    for s in &c.subdomain {
        *keys.entry(reversed(s)).or_default() |= SUBDOMAIN_BIT;
    }
    let mut fst = fst::MapBuilder::memory();
    for (k, v) in &keys {
        fst.insert(k, *v).expect("sorted unique keys");
    }
    let fst = fst.into_inner().expect("fst");

    let mut kw = vec![];
    kw.extend((c.keyword.len() as u32).to_le_bytes());
    let mut offset = 0u32;
    kw.extend(offset.to_le_bytes());
    for k in &c.keyword {
        offset += k.len() as u32;
        kw.extend(offset.to_le_bytes());
    }
    for k in &c.keyword {
        kw.extend(k.as_bytes());
    }
    let mut v4 = Vec::with_capacity(c.v4.len() * 8);
    for (s, e) in &c.v4 {
        v4.extend(s.to_le_bytes());
        v4.extend(e.to_le_bytes());
    }
    let mut v6 = Vec::with_capacity(c.v6.len() * 32);
    for (s, e) in &c.v6 {
        v6.extend(s.to_le_bytes());
        v6.extend(e.to_le_bytes());
    }
    let mut sections: Vec<(u32, Vec<u8>)> =
        vec![(DOMAINS, fst), (KEYWORDS, kw), (IPV4, v4), (IPV6, v6)];
    if hashed {
        sections.push((DOMAIN_HASH, hash_index(&keys)));
    }
    let mut out = vec![0u8; HEADER + SECTION * sections.len()];
    let mut table = vec![];
    for (kind, body) in &sections {
        while out.len() % 8 != 0 {
            out.push(0);
        }
        table.push((*kind, out.len() as u64, body.len() as u64));
        out.extend(body);
    }
    out[0..8].copy_from_slice(MAGIC);
    out[8..10].copy_from_slice(&MAJOR.to_le_bytes());
    out[16..20].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    let total = out.len() as u64;
    out[24..32].copy_from_slice(&total.to_le_bytes());
    for (i, n) in [
        c.exact.len(),
        c.suffix.len(),
        c.keyword.len(),
        c.v4.len(),
        c.v6.len(),
        c.subdomain.len(),
    ]
    .iter()
    .enumerate()
    {
        out[32 + i * 4..36 + i * 4].copy_from_slice(&(*n as u32).to_le_bytes());
    }
    for (i, (kind, off, len)) in table.iter().enumerate() {
        let at = HEADER + i * SECTION;
        out[at..at + 4].copy_from_slice(&kind.to_le_bytes());
        let flags = if *kind == DOMAIN_HASH { 0 } else { REQUIRED };
        out[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&off.to_le_bytes());
        out[at + 16..at + 24].copy_from_slice(&len.to_le_bytes());
    }
    out
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// `{ n u32, mask u32, slots[mask+1] {fp u32, idx+1 u32}, offsets[n+1] u32, kinds[n] u8, blob }`;
/// keys are reversed names hashed incrementally with FNV-1a.
fn hash_index(keys: &BTreeMap<Vec<u8>, u64>) -> Vec<u8> {
    let n = keys.len();
    let size = (n * 2).max(2).next_power_of_two();
    let mask = size - 1;
    let mut slots = vec![(0u32, 0u32); size];
    let mut offsets = vec![0u32];
    let mut kinds: Vec<u8> = vec![];
    let mut blob: Vec<u8> = vec![];
    for (i, (k, v)) in keys.iter().enumerate() {
        let mut h = FNV_OFFSET;
        for b in k {
            h = (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME);
        }
        let mut at = (h as usize) & mask;
        while slots[at].1 != 0 {
            at = (at + 1) & mask;
        }
        slots[at] = ((h >> 32) as u32, i as u32 + 1);
        blob.extend(k);
        offsets.push(blob.len() as u32);
        kinds.push(*v as u8);
    }
    let mut out = vec![];
    out.extend((n as u32).to_le_bytes());
    out.extend((mask as u32).to_le_bytes());
    for (fp, idx) in slots {
        out.extend(fp.to_le_bytes());
        out.extend(idx.to_le_bytes());
    }
    for o in offsets {
        out.extend(o.to_le_bytes());
    }
    out.extend(kinds);
    out.extend(blob);
    out
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Any immutable byte backing: a heap buffer or a read-only mapping.
pub type Backing = Arc<dyn AsRef<[u8]> + Send + Sync>;

#[derive(Clone)]
pub struct Slice {
    backing: Backing,
    range: Range<usize>,
}
impl AsRef<[u8]> for Slice {
    fn as_ref(&self) -> &[u8] {
        &(*self.backing).as_ref()[self.range.clone()]
    }
}

#[derive(Debug)]
pub struct Invalid(pub &'static str);

pub struct RuleSet {
    domains: fst::Map<Slice>,
    hashed: Option<Slice>,
    keywords: Option<AhoCorasick>,
    v4: Slice,
    v6: Slice,
    #[allow(dead_code)]
    pub counts: [u32; 6],
}

#[derive(Debug, PartialEq, Eq)]
pub enum DomainHit {
    Exact,
    /// Matched suffix is the trailing `len` bytes of the queried name.
    Suffix {
        #[allow(dead_code)]
        len: usize,
    },
    Keyword,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}
fn u128_at(b: &[u8], at: usize) -> u128 {
    u128::from_le_bytes(b[at..at + 16].try_into().expect("16 bytes"))
}

impl RuleSet {
    /// Structural checks only; content integrity is the caller's digest check.
    pub fn open(backing: Backing) -> Result<Self, Invalid> {
        let bytes = (*backing).as_ref();
        if bytes.len() < HEADER || &bytes[0..8] != MAGIC {
            return Err(Invalid("magic"));
        }
        if u16::from_le_bytes([bytes[8], bytes[9]]) != MAJOR {
            return Err(Invalid("major"));
        }
        if u64_at(bytes, 24) != bytes.len() as u64 {
            return Err(Invalid("length"));
        }
        let count = u32_at(bytes, 16) as usize;
        if count > 64 || HEADER + count * SECTION > bytes.len() {
            return Err(Invalid("section table"));
        }
        let counts: [u32; 6] = std::array::from_fn(|i| u32_at(bytes, 32 + i * 4));
        let mut found: BTreeMap<u32, Range<usize>> = BTreeMap::new();
        for i in 0..count {
            let at = HEADER + i * SECTION;
            let kind = u32_at(bytes, at);
            let flags = u32_at(bytes, at + 4);
            let off = u64_at(bytes, at + 8) as usize;
            let len = u64_at(bytes, at + 16) as usize;
            if off % 8 != 0 || off.checked_add(len).is_none_or(|end| end > bytes.len()) {
                return Err(Invalid("section bounds"));
            }
            match kind {
                DOMAINS | KEYWORDS | IPV4 | IPV6 | DOMAIN_HASH => {
                    if found.insert(kind, off..off + len).is_some() {
                        return Err(Invalid("duplicate section"));
                    }
                }
                _ if flags & REQUIRED != 0 => return Err(Invalid("unknown required section")),
                _ => {}
            }
        }
        let slice = |kind| Slice {
            backing: backing.clone(),
            range: found.get(&kind).cloned().unwrap_or(0..0),
        };
        let domains = fst::Map::new(slice(DOMAINS)).map_err(|_| Invalid("fst"))?;
        if domains.len() as u64 > u64::from(counts[0]) + u64::from(counts[1]) + u64::from(counts[5])
        {
            return Err(Invalid("domain count"));
        }
        let v4 = slice(IPV4);
        let v6 = slice(IPV6);
        if v4.as_ref().len() != counts[3] as usize * 8
            || v6.as_ref().len() != counts[4] as usize * 32
        {
            return Err(Invalid("ip count"));
        }
        let kw = slice(KEYWORDS);
        let keywords = Self::keywords(kw.as_ref(), counts[2])?;
        let hashed = found.contains_key(&DOMAIN_HASH).then(|| slice(DOMAIN_HASH));
        if let Some(h) = &hashed {
            let b = h.as_ref();
            let ok = b.len() >= 8 && {
                let n = u32_at(b, 0) as usize;
                let slots = u32_at(b, 4) as usize + 1;
                let need = 8 + slots * 8 + (n + 1) * 4 + n;
                slots.is_power_of_two()
                    && need <= b.len()
                    && u32_at(b, 8 + slots * 8 + n * 4) as usize == b.len() - need
            };
            if !ok {
                return Err(Invalid("hash index"));
            }
        }
        Ok(Self {
            domains,
            hashed,
            keywords,
            v4,
            v6,
            counts,
        })
    }

    fn keywords(b: &[u8], expect: u32) -> Result<Option<AhoCorasick>, Invalid> {
        if b.len() < 8 || u32_at(b, 0) != expect {
            return Err(Invalid("keywords"));
        }
        let n = expect as usize;
        if n == 0 {
            return Ok(None);
        }
        let blob = 4 + (n + 1) * 4;
        if blob > b.len() {
            return Err(Invalid("keyword table"));
        }
        let mut words = Vec::with_capacity(n);
        for i in 0..n {
            let s = u32_at(b, 4 + i * 4) as usize;
            let e = u32_at(b, 8 + i * 4) as usize;
            words.push(b.get(blob + s..blob + e).ok_or(Invalid("keyword bounds"))?);
        }
        AhoCorasick::builder()
            .kind(Some(AhoCorasickKind::DFA))
            .match_kind(MatchKind::LeftmostFirst)
            .build(words)
            .map(Some)
            .map_err(|_| Invalid("keyword automaton"))
    }

    /// O(n) checks a loader may skip for digest-verified artifacts.
    pub fn verify(&self) -> Result<(), Invalid> {
        self.domains
            .as_fst()
            .verify()
            .map_err(|_| Invalid("fst checksum"))?;
        let v4 = self.v4.as_ref();
        let mut prev: Option<u32> = None;
        for i in 0..v4.len() / 8 {
            let (s, e) = (u32_at(v4, i * 8), u32_at(v4, i * 8 + 4));
            if s > e || prev.is_some_and(|p| s <= p) {
                return Err(Invalid("v4 order"));
            }
            prev = Some(e);
        }
        let v6 = self.v6.as_ref();
        let mut prev: Option<u128> = None;
        for i in 0..v6.len() / 32 {
            let (s, e) = (u128_at(v6, i * 32), u128_at(v6, i * 32 + 16));
            if s > e || prev.is_some_and(|p| s <= p) {
                return Err(Invalid("v6 order"));
            }
            prev = Some(e);
        }
        Ok(())
    }

    /// `name` must be canonical (lowercase ASCII, no trailing dot).
    pub fn match_domain(&self, name: &[u8]) -> Option<DomainHit> {
        let fst = self.domains.as_fst();
        let mut node = fst.root();
        let mut out = fst::raw::Output::zero();
        let n = name.len();
        for i in (0..n).rev() {
            let Some(t) = node.find_input(name[i]) else {
                break;
            };
            let t = node.transition(t);
            out = out.cat(t.out);
            node = fst.node(t.addr);
            if node.is_final() && (i == 0 || name[i - 1] == b'.') {
                let v = out.cat(node.final_output()).value();
                if v & SUFFIX_BIT != 0 || (i > 0 && v & SUBDOMAIN_BIT != 0) {
                    return Some(DomainHit::Suffix { len: n - i });
                }
                if i == 0 && v & EXACT_BIT != 0 {
                    return Some(DomainHit::Exact);
                }
            }
        }
        self.keywords
            .as_ref()
            .filter(|ac| ac.is_match(name))
            .map(|_| DomainHit::Keyword)
    }

    /// Same semantics as `match_domain` through the optional hash index.
    pub fn match_domain_hashed(&self, name: &[u8]) -> Option<DomainHit> {
        let b = self.hashed.as_ref()?.as_ref();
        let n = u32_at(b, 0) as usize;
        let mask = u32_at(b, 4) as usize;
        let slots = 8;
        let offsets = slots + (mask + 1) * 8;
        let kinds = offsets + (n + 1) * 4;
        let blob = kinds + n;
        let len = name.len();
        let mut h = FNV_OFFSET;
        for i in (0..len).rev() {
            h = (h ^ u64::from(name[i])).wrapping_mul(FNV_PRIME);
            if i != 0 && name[i - 1] != b'.' {
                continue;
            }
            let fp = (h >> 32) as u32;
            let mut at = (h as usize) & mask;
            loop {
                let s = slots + at * 8;
                let idx = u32_at(b, s + 4) as usize;
                if idx == 0 {
                    break;
                }
                if u32_at(b, s) == fp {
                    let e = idx - 1;
                    let (start, end) = (
                        u32_at(b, offsets + e * 4) as usize,
                        u32_at(b, offsets + e * 4 + 4) as usize,
                    );
                    let key = &b[blob + start..blob + end];
                    if key.len() == len - i && key.iter().eq(name[i..].iter().rev()) {
                        let v = u64::from(b[kinds + e]);
                        if v & SUFFIX_BIT != 0 || (i > 0 && v & SUBDOMAIN_BIT != 0) {
                            return Some(DomainHit::Suffix { len: len - i });
                        }
                        if i == 0 && v & EXACT_BIT != 0 {
                            return Some(DomainHit::Exact);
                        }
                        break;
                    }
                }
                at = (at + 1) & mask;
            }
        }
        self.keywords
            .as_ref()
            .filter(|ac| ac.is_match(name))
            .map(|_| DomainHit::Keyword)
    }

    pub fn match_ip(&self, addr: IpAddr) -> bool {
        match addr {
            IpAddr::V4(a) => {
                let a = u32::from(a);
                let b = self.v4.as_ref();
                let n = b.len() / 8;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if u32_at(b, mid * 8) <= a {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                lo > 0 && u32_at(b, (lo - 1) * 8 + 4) >= a
            }
            IpAddr::V6(a) => {
                let a = u128::from(a);
                let b = self.v6.as_ref();
                let n = b.len() / 32;
                let (mut lo, mut hi) = (0, n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    if u128_at(b, mid * 32) <= a {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                lo > 0 && u128_at(b, (lo - 1) * 32 + 16) >= a
            }
        }
    }
}

/// Reference matcher used to cross-check the artifact on every benchmark query.
pub struct Reference<'a> {
    pub c: &'a Canonical,
}
impl Reference<'_> {
    pub fn domain(&self, name: &str) -> bool {
        self.c.exact.contains(name)
            || label_suffixes(name).any(|s| self.c.suffix.contains(s))
            || label_suffixes(name)
                .skip(1)
                .any(|s| self.c.subdomain.contains(s))
            || self.c.keyword.iter().any(|k| name.contains(k.as_str()))
    }
    pub fn ip(&self, addr: IpAddr) -> bool {
        match addr {
            IpAddr::V4(a) => {
                let a = u32::from(a);
                self.c.v4.iter().any(|(s, e)| *s <= a && a <= *e)
            }
            IpAddr::V6(a) => {
                let a = u128::from(a);
                self.c.v6.iter().any(|(s, e)| *s <= a && a <= *e)
            }
        }
    }
}

/// Minimum CIDR blocks covering the merged v4 ranges (what a kernel route
/// table would need for capture partitioning).
pub fn v4_prefix_count(ranges: &[(u32, u32)]) -> usize {
    let mut n = 0;
    for &(s, e) in ranges {
        let (mut s, e) = (u64::from(s), u64::from(e) + 1);
        while s < e {
            let mut size = if s == 0 {
                1u64 << 32
            } else {
                s & s.wrapping_neg()
            };
            while s + size > e {
                size >>= 1;
            }
            s += size;
            n += 1;
        }
    }
    n
}
