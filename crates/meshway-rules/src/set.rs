use crate::{
    Digest,
    domains::{self, Fst, Keys},
    format::{
        Counts, DOMAINS, EXACT, HEADER, IPV4, IPV6, KEYWORDS, MAGIC, MAJOR, MAX_ARTIFACT_BYTES,
        MAX_KEYWORDS, MAX_NAMES, MAX_RANGES, MAX_SECTIONS, REQUIRED, SECTION, SUBDOMAIN, SUFFIX,
        u32_at, u64_at, u128_at,
    },
    name::keyword_byte,
};
use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ops::{Range, RangeInclusive},
};

pub(crate) enum Backing {
    Heap(Vec<u8>),
    #[cfg(feature = "mapped")]
    Mapped(memmap2::Mmap),
}

impl Backing {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Heap(v) => v,
            #[cfg(feature = "mapped")]
            Self::Mapped(m) => m,
        }
    }
}

/// A verified artifact. Opening checks the header, section table, every
/// section's structure and computes the digest; afterwards matching reads the
/// bytes in place. Share one handle per digest per process (`Arc<RuleSet>`).
pub struct RuleSet {
    backing: Backing,
    digest: Digest,
    minor: u16,
    counts: Counts,
    domains: Range<usize>,
    keyword_table: Range<usize>,
    keywords: Option<AhoCorasick>,
    v4: Range<usize>,
    v6: Range<usize>,
}

/// Why bytes are not a usable `MWRS` artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    #[error("The artifact exceeds {MAX_ARTIFACT_BYTES} bytes.")]
    TooLarge,
    #[error("Unsupported rule-set format major version {0}.")]
    UnsupportedMajor(u16),
    #[error("Invalid rule-set artifact: {0}.")]
    Invalid(&'static str),
}

fn invalid(reason: &'static str) -> LoadError {
    LoadError::Invalid(reason)
}

/// What a domain member is; see the matching table in the design.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DomainKind {
    Exact,
    Suffix,
    Subdomain,
}

/// The member that matched. Lengths count bytes at the end of the query, so
/// `&query[query.len() - len..]` is the member name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DomainHit {
    Exact,
    Suffix {
        len: usize,
    },
    Subdomain {
        len: usize,
    },
    /// Index into [`RuleSet::keywords`].
    Keyword {
        index: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct DomainEntry {
    pub kind: DomainKind,
    pub name: String,
}

struct Section {
    kind: u32,
    range: Range<usize>,
}

/// One section-table entry as stored, including ignored optional sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionInfo {
    pub kind: u32,
    pub required: bool,
    pub offset: u64,
    pub length: u64,
}

impl RuleSet {
    /// Heap backend: reads the artifact from an owned buffer.
    pub fn from_vec(bytes: Vec<u8>) -> Result<Self, LoadError> {
        Self::open(Backing::Heap(bytes))
    }

    pub(crate) fn open(backing: Backing) -> Result<Self, LoadError> {
        let bytes = backing.bytes();
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(LoadError::TooLarge);
        }
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err(invalid("magic"));
        }
        let major = u16::from_le_bytes([bytes[8], bytes[9]]);
        if major != MAJOR {
            return Err(LoadError::UnsupportedMajor(major));
        }
        let minor = u16::from_le_bytes([bytes[10], bytes[11]]);
        let header_u32 = |at| u32_at(bytes, at).ok_or(invalid("header"));
        if header_u32(12)? != 0 || header_u32(20)? != 0 || bytes[56..64].iter().any(|b| *b != 0) {
            return Err(invalid("reserved header bytes"));
        }
        if u64_at(bytes, 24) != Some(bytes.len() as u64) {
            return Err(invalid("total length"));
        }
        let counts = Counts::from_array(std::array::from_fn(|i| {
            u32_at(bytes, 32 + i * 4).unwrap_or(u32::MAX)
        }));
        if counts.names() > MAX_NAMES as u64
            || counts.keyword as usize > MAX_KEYWORDS
            || counts.ipv4 as usize > MAX_RANGES
            || counts.ipv6 as usize > MAX_RANGES
        {
            return Err(invalid("counts exceed limits"));
        }
        let sections = Self::section_table(bytes)?;
        let find = |kind| -> Result<Range<usize>, LoadError> {
            let mut found = sections.iter().filter(|s| s.kind == kind);
            match (found.next(), found.next()) {
                (Some(s), None) => Ok(s.range.clone()),
                (None, _) => Err(invalid("missing section")),
                (Some(_), Some(_)) => Err(invalid("duplicate section")),
            }
        };
        let (domains, keyword_table, v4, v6) =
            (find(DOMAINS)?, find(KEYWORDS)?, find(IPV4)?, find(IPV6)?);

        let fst = Fst::parse(&bytes[domains.clone()]).ok_or(invalid("domain index"))?;
        if fst.len() != counts.names() {
            return Err(invalid("domain count"));
        }
        domains::validate(&fst).ok_or(invalid("domain index"))?;
        let keywords = Self::keyword_section(&bytes[keyword_table.clone()], counts.keyword)?;
        Self::ranges::<4>(&bytes[v4.clone()], counts.ipv4)?;
        Self::ranges::<16>(&bytes[v6.clone()], counts.ipv6)?;
        let digest = Digest::of(bytes);
        Ok(Self {
            backing,
            digest,
            minor,
            counts,
            domains,
            keyword_table,
            keywords,
            v4,
            v6,
        })
    }

    /// Section table: aligned, in bounds, non-overlapping, gaps zero; unknown
    /// optional sections are ignored and unknown required ones rejected.
    fn section_table(bytes: &[u8]) -> Result<Vec<Section>, LoadError> {
        let count = u32_at(bytes, 16).ok_or(invalid("header"))? as usize;
        let table_end = count
            .checked_mul(SECTION)
            .and_then(|n| n.checked_add(HEADER))
            .filter(|end| count <= MAX_SECTIONS && *end <= bytes.len())
            .ok_or(invalid("section table"))?;
        let mut sections = Vec::with_capacity(count);
        let mut occupied = Vec::with_capacity(count + 1);
        occupied.push(0..table_end);
        for i in 0..count {
            let at = HEADER + i * SECTION;
            let field = |off| u64_at(bytes, at + off).ok_or(invalid("section table"));
            let kind = u32_at(bytes, at).ok_or(invalid("section table"))?;
            let flags = u32_at(bytes, at + 4).ok_or(invalid("section table"))?;
            let offset = usize::try_from(field(8)?).map_err(|_| invalid("section bounds"))?;
            let length = usize::try_from(field(16)?).map_err(|_| invalid("section bounds"))?;
            let end = offset
                .checked_add(length)
                .filter(|end| offset % 8 == 0 && offset >= table_end && *end <= bytes.len())
                .ok_or(invalid("section bounds"))?;
            if flags & !REQUIRED != 0 {
                return Err(invalid("section flags"));
            }
            let known = matches!(kind, DOMAINS | KEYWORDS | IPV4 | IPV6);
            if known && flags != REQUIRED {
                return Err(invalid("section flags"));
            }
            if !known && flags & REQUIRED != 0 {
                return Err(invalid("unknown required section"));
            }
            occupied.push(offset..end);
            if known {
                sections.push(Section {
                    kind,
                    range: offset..end,
                });
            }
        }
        occupied.sort_by_key(|r| (r.start, r.end));
        let mut cursor = 0;
        for r in &occupied {
            if r.start < cursor {
                return Err(invalid("overlapping sections"));
            }
            if bytes[cursor..r.start].iter().any(|b| *b != 0) {
                return Err(invalid("nonzero padding"));
            }
            cursor = r.end;
        }
        if bytes[cursor..].iter().any(|b| *b != 0) {
            return Err(invalid("nonzero padding"));
        }
        Ok(sections)
    }

    /// `count u32`, `count + 1` ascending u32 offsets, blob; literals strictly
    /// sorted, nonempty, name bytes only.
    fn keyword_section(b: &[u8], count: u32) -> Result<Option<AhoCorasick>, LoadError> {
        let n = count as usize;
        if u32_at(b, 0) != Some(count) {
            return Err(invalid("keyword count"));
        }
        let blob = 4 + (n + 1) * 4;
        if u32_at(b, 4) != Some(0)
            || u32_at(b, blob - 4).map(|end| blob + end as usize) != Some(b.len())
        {
            return Err(invalid("keyword table"));
        }
        let mut words: Vec<&[u8]> = Vec::with_capacity(n);
        for i in 0..n {
            let start = u32_at(b, 4 + i * 4).ok_or(invalid("keyword table"))? as usize;
            let end = u32_at(b, 8 + i * 4).ok_or(invalid("keyword table"))? as usize;
            let word = (start < end)
                .then(|| b.get(blob + start..blob + end))
                .flatten()
                .filter(|w| w.len() <= 253 && w.iter().all(|c| keyword_byte(*c)))
                .ok_or(invalid("keyword table"))?;
            if words.last().is_some_and(|prev| *prev >= word) {
                return Err(invalid("keyword order"));
            }
            words.push(word);
        }
        if words.is_empty() {
            return Ok(None);
        }
        AhoCorasick::builder()
            .kind(Some(AhoCorasickKind::DFA))
            .match_kind(MatchKind::LeftmostFirst)
            .build(&words)
            .map(Some)
            .map_err(|_| invalid("keyword automaton"))
    }

    /// `count` closed `(start, end)` pairs of `W`-byte integers, sorted,
    /// disjoint and not adjacent.
    fn ranges<const W: usize>(b: &[u8], count: u32) -> Result<(), LoadError> {
        if b.len() != count as usize * W * 2 {
            return Err(invalid("range count"));
        }
        let mut previous: Option<u128> = None;
        for i in 0..count as usize {
            let pair = &b[i * W * 2..(i + 1) * W * 2];
            let (start, end) = (int::<W>(&pair[..W]), int::<W>(&pair[W..]));
            let separated = previous.is_none_or(|p| p.checked_add(1).is_some_and(|n| start > n));
            if start > end || !separated {
                return Err(invalid("range order"));
            }
            previous = Some(end);
        }
        Ok(())
    }

    fn bytes(&self) -> &[u8] {
        self.backing.bytes()
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes()
    }
    pub fn digest(&self) -> Digest {
        self.digest
    }
    pub fn minor(&self) -> u16 {
        self.minor
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    /// The section table in stored order.
    pub fn sections(&self) -> Vec<SectionInfo> {
        let b = self.bytes();
        let count = u32_at(b, 16).unwrap_or(0) as usize;
        (0..count)
            .filter_map(|i| {
                let at = HEADER + i * SECTION;
                Some(SectionInfo {
                    kind: u32_at(b, at)?,
                    required: u32_at(b, at + 4)? & REQUIRED != 0,
                    offset: u64_at(b, at + 8)?,
                    length: u64_at(b, at + 16)?,
                })
            })
            .collect()
    }
    fn fst(&self) -> Fst<'_> {
        Fst::parse(&self.bytes()[self.domains.clone()]).expect("validated at open")
    }

    /// `name` must be a canonical query name (lowercase ASCII, no trailing
    /// dot; for example `Name::as_str`). Bytes are compared literally, so any
    /// other spelling does not match. Suffix and subdomain members are tried
    /// from the shortest; an exact member only at the full name; keywords last.
    pub fn match_domain(&self, name: &str) -> Option<DomainHit> {
        let name = name.as_bytes();
        let fst = self.fst();
        if let Some(mut node) = fst.root() {
            let mut sum = 0u64;
            for i in (0..name.len()).rev() {
                let Some(t) = fst.find(&node, name[i]) else {
                    break;
                };
                let Some(next) = fst.node(t.target) else {
                    break;
                };
                sum = sum.wrapping_add(t.output);
                node = next;
                if node.is_final && (i == 0 || name[i - 1] == b'.') {
                    let len = name.len() - i;
                    match sum.wrapping_add(node.final_output) {
                        SUFFIX => return Some(DomainHit::Suffix { len }),
                        SUBDOMAIN if i > 0 => return Some(DomainHit::Subdomain { len }),
                        EXACT if i == 0 => return Some(DomainHit::Exact),
                        _ => {}
                    }
                }
            }
        }
        let m = self.keywords.as_ref()?.find(name)?;
        Some(DomainHit::Keyword {
            index: m.pattern().as_usize(),
        })
    }

    pub fn match_ip(&self, addr: IpAddr) -> bool {
        match addr {
            IpAddr::V4(a) => self.contains::<4>(&self.v4, u128::from(u32::from(a))),
            IpAddr::V6(a) => self.contains::<16>(&self.v6, u128::from(a)),
        }
    }

    fn contains<const W: usize>(&self, section: &Range<usize>, addr: u128) -> bool {
        let b = &self.bytes()[section.clone()];
        let n = b.len() / (W * 2);
        let at = |i: usize, half: usize| int::<W>(&b[i * W * 2 + half * W..][..W]);
        let (mut lo, mut hi) = (0, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if at(mid, 0) <= addr {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo > 0 && at(lo - 1, 1) >= addr
    }

    /// Merged ranges in ascending order: disjoint, non-adjacent, closed. This
    /// is directly an nftables interval set.
    pub fn ipv4_ranges(&self) -> impl ExactSizeIterator<Item = RangeInclusive<Ipv4Addr>> + '_ {
        self.bytes()[self.v4.clone()]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                Ipv4Addr::from(int::<4>(&p[..4]) as u32)..=Ipv4Addr::from(int::<4>(&p[4..]) as u32)
            })
    }
    pub fn ipv6_ranges(&self) -> impl ExactSizeIterator<Item = RangeInclusive<Ipv6Addr>> + '_ {
        self.bytes()[self.v6.clone()]
            .as_chunks::<32>()
            .0
            .iter()
            .map(|p| Ipv6Addr::from(int::<16>(&p[..16]))..=Ipv6Addr::from(int::<16>(&p[16..])))
    }

    /// Keyword literals in stored (sorted) order.
    pub fn keywords(&self) -> impl ExactSizeIterator<Item = &str> + '_ {
        (0..self.counts.keyword as usize).map(|i| self.keyword(i).expect("validated at open"))
    }
    pub fn keyword(&self, index: usize) -> Option<&str> {
        let b = &self.bytes()[self.keyword_table.clone()];
        let n = self.counts.keyword as usize;
        if index >= n {
            return None;
        }
        let blob = 4 + (n + 1) * 4;
        let start = u32_at(b, 4 + index * 4)? as usize;
        let end = u32_at(b, 8 + index * 4)? as usize;
        std::str::from_utf8(b.get(blob + start..blob + end)?).ok()
    }

    /// Every exact／suffix／subdomain member, ordered by reversed name.
    pub fn domains(&self) -> impl Iterator<Item = DomainEntry> + '_ {
        Keys::new(self.fst()).filter_map(|(mut key, value)| {
            let kind = match value {
                EXACT => DomainKind::Exact,
                SUFFIX => DomainKind::Suffix,
                SUBDOMAIN => DomainKind::Subdomain,
                _ => return None,
            };
            key.reverse();
            Some(DomainEntry {
                kind,
                name: String::from_utf8(key).ok()?,
            })
        })
    }
}

fn int<const W: usize>(b: &[u8]) -> u128 {
    match W {
        4 => u128::from(u32_at(b, 0).unwrap_or(0)),
        _ => u128_at(b, 0).unwrap_or(0),
    }
}

impl std::fmt::Debug for RuleSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleSet")
            .field("digest", &self.digest)
            .field("bytes", &self.bytes().len())
            .field("counts", &self.counts)
            .finish()
    }
}
