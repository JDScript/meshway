use crate::{
    Digest, Entry, RuleSet,
    format::{
        self, Counts, DOMAINS, EXACT, IPV4, IPV6, KEYWORDS, MAX_ARTIFACT_BYTES, MAX_KEYWORDS,
        MAX_NAMES, MAX_RANGES, SUBDOMAIN, SUFFIX,
    },
    set::LoadError,
};
use aho_corasick::{AhoCorasick, MatchKind};
use std::collections::{BTreeMap, BTreeSet};

/// Collects normalized entries; [`Builder::build`] minimizes them and writes
/// the only byte string for that set.
#[derive(Default)]
pub struct Builder {
    exact: BTreeSet<String>,
    suffix: BTreeSet<String>,
    subdomain: BTreeSet<String>,
    keyword: BTreeSet<String>,
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
    /// Name and keyword insertions, duplicates included.
    inserted: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Limit {
    Bytes,
    Names,
    Keywords,
    Ipv4Ranges,
    Ipv6Ranges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("The rule set exceeds the {limit:?} limit of {max} (has {actual}).")]
pub struct BuildError {
    pub limit: Limit,
    pub max: u64,
    pub actual: u64,
}

/// A built artifact and the facts a catalog or resource record states about it.
#[derive(Clone, Debug)]
pub struct Artifact {
    bytes: Vec<u8>,
    digest: Digest,
    counts: Counts,
    redundant: u64,
}

impl Artifact {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    /// Name and keyword insertions dropped as duplicates or covered by
    /// another member; merged IP prefixes are not counted.
    pub fn redundant(&self) -> u64 {
        self.redundant
    }
    pub fn open(self) -> Result<RuleSet, LoadError> {
        RuleSet::from_vec(self.bytes)
    }
}

fn label_suffixes(name: &str) -> impl Iterator<Item = &str> {
    std::iter::once(name).chain(name.match_indices('.').map(move |(i, _)| &name[i + 1..]))
}

macro_rules! merge {
    ($name:ident, $t:ty) => {
        fn $name(mut ranges: Vec<($t, $t)>) -> Vec<($t, $t)> {
            ranges.sort_unstable();
            let mut out: Vec<($t, $t)> = Vec::with_capacity(ranges.len());
            for (start, end) in ranges {
                match out.last_mut() {
                    Some(last) if start <= last.1 || last.1.checked_add(1) == Some(start) => {
                        last.1 = last.1.max(end);
                    }
                    _ => out.push((start, end)),
                }
            }
            out
        }
    };
}
merge!(merge4, u32);
merge!(merge6, u128);

fn budget(limit: Limit, max: usize, actual: usize) -> Result<(), BuildError> {
    if actual > max {
        return Err(BuildError {
            limit,
            max: max as u64,
            actual: actual as u64,
        });
    }
    Ok(())
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, entry: Entry) {
        match entry {
            Entry::Exact(n) => self.name(n.into_string(), EXACT),
            Entry::Suffix(n) => self.name(n.into_string(), SUFFIX),
            Entry::Subdomain(n) => self.name(n.into_string(), SUBDOMAIN),
            Entry::Keyword(k) => {
                self.inserted += 1;
                self.keyword.insert(k.as_str().to_owned());
            }
            Entry::Cidr(c) => {
                if let Some(r) = c.v4_range() {
                    self.v4.push(r);
                }
                if let Some(r) = c.v6_range() {
                    self.v6.push(r);
                }
            }
        }
    }

    fn name(&mut self, name: String, kind: u64) {
        self.inserted += 1;
        match kind {
            EXACT => self.exact.insert(name),
            SUFFIX => self.suffix.insert(name),
            _ => self.subdomain.insert(name),
        };
    }

    pub fn build(self) -> Result<Artifact, BuildError> {
        let inserted = self.inserted;
        let raw_keywords = self.keyword.len();
        let canonical = self.minimize().ok_or(BuildError {
            limit: Limit::Keywords,
            max: MAX_KEYWORDS as u64,
            actual: raw_keywords as u64,
        })?;
        let counts = Counts {
            exact: canonical.exact.len() as u32,
            suffix: canonical.suffix.len() as u32,
            subdomain: canonical.subdomain.len() as u32,
            keyword: canonical.keyword.len() as u32,
            ipv4: canonical.v4.len() as u32,
            ipv6: canonical.v6.len() as u32,
        };
        budget(
            Limit::Names,
            MAX_NAMES,
            canonical.exact.len() + canonical.suffix.len() + canonical.subdomain.len(),
        )?;
        budget(Limit::Keywords, MAX_KEYWORDS, canonical.keyword.len())?;
        budget(Limit::Ipv4Ranges, MAX_RANGES, canonical.v4.len())?;
        budget(Limit::Ipv6Ranges, MAX_RANGES, canonical.v6.len())?;
        let bytes = canonical.encode(counts);
        budget(Limit::Bytes, MAX_ARTIFACT_BYTES, bytes.len())?;
        Ok(Artifact {
            digest: Digest::of(&bytes),
            bytes,
            counts,
            redundant: inserted - (counts.names() + u64::from(counts.keyword)),
        })
    }

    /// Removes every member another member already covers, so the stored set
    /// is a function of the inserted set alone:
    ///
    /// - a keyword containing another keyword;
    /// - a name containing a keyword (suffix and subdomain members only
    ///   match names containing themselves);
    /// - `exact(n)` with `subdomain(n)` becomes `suffix(n)`;
    /// - a name under a suffix at a label boundary (the suffix itself
    ///   included for exact／subdomain members), or strictly under a
    ///   subdomain member.
    ///
    /// `None` only when the keyword automaton exceeds its own size limits.
    fn minimize(self) -> Option<Canonical> {
        let Self {
            mut exact,
            mut suffix,
            mut subdomain,
            keyword,
            v4,
            v6,
            ..
        } = self;
        let all: Vec<&String> = keyword.iter().collect();
        let keyword: BTreeSet<String> = if all.is_empty() {
            BTreeSet::new()
        } else {
            let ac = AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&all)
                .ok()?;
            all.iter()
                .enumerate()
                .filter(|(i, k)| {
                    !ac.find_overlapping_iter(k.as_str())
                        .any(|m| m.pattern().as_usize() != *i)
                })
                .map(|(_, k)| (*k).clone())
                .collect()
        };
        let ac = match keyword.is_empty() {
            true => None,
            false => Some(AhoCorasick::new(&keyword).ok()?),
        };
        let has_keyword = |n: &str| ac.as_ref().is_some_and(|a| a.is_match(n));

        let promoted: Vec<String> = exact.intersection(&subdomain).cloned().collect();
        for n in promoted {
            exact.remove(&n);
            subdomain.remove(&n);
            suffix.insert(n);
        }
        let covered = |n: &str, itself: bool| {
            label_suffixes(n).enumerate().any(|(i, t)| {
                (suffix.contains(t) && (i > 0 || itself)) || (i > 0 && subdomain.contains(t))
            })
        };
        let keep = |set: &BTreeSet<String>, itself: bool| -> BTreeSet<String> {
            set.iter()
                .filter(|n| !covered(n, itself) && !has_keyword(n))
                .cloned()
                .collect()
        };
        Some(Canonical {
            exact: keep(&exact, true),
            suffix: keep(&suffix, false),
            subdomain: keep(&subdomain, true),
            keyword,
            v4: merge4(v4),
            v6: merge6(v6),
        })
    }
}

struct Canonical {
    exact: BTreeSet<String>,
    suffix: BTreeSet<String>,
    subdomain: BTreeSet<String>,
    keyword: BTreeSet<String>,
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}

impl Canonical {
    fn encode(&self, counts: Counts) -> Vec<u8> {
        let mut keys: BTreeMap<Vec<u8>, u64> = BTreeMap::new();
        for (set, bit) in [
            (&self.exact, EXACT),
            (&self.suffix, SUFFIX),
            (&self.subdomain, SUBDOMAIN),
        ] {
            for n in set {
                keys.insert(n.bytes().rev().collect(), bit);
            }
        }
        let mut fst = fst::MapBuilder::memory();
        for (k, v) in &keys {
            fst.insert(k, *v).expect("sorted unique keys");
        }
        let fst = fst.into_inner().expect("in-memory fst");

        let mut kw = Vec::with_capacity(8 + self.keyword.len() * 16);
        kw.extend((self.keyword.len() as u32).to_le_bytes());
        let mut offset = 0u32;
        kw.extend(offset.to_le_bytes());
        for k in &self.keyword {
            offset += k.len() as u32;
            kw.extend(offset.to_le_bytes());
        }
        for k in &self.keyword {
            kw.extend(k.as_bytes());
        }
        let mut v4 = Vec::with_capacity(self.v4.len() * 8);
        for (s, e) in &self.v4 {
            v4.extend(s.to_le_bytes());
            v4.extend(e.to_le_bytes());
        }
        let mut v6 = Vec::with_capacity(self.v6.len() * 32);
        for (s, e) in &self.v6 {
            v6.extend(s.to_le_bytes());
            v6.extend(e.to_le_bytes());
        }
        format::assemble(
            counts,
            &[(DOMAINS, &fst), (KEYWORDS, &kw), (IPV4, &v4), (IPV6, &v6)],
        )
    }
}
