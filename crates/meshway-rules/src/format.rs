//! `MWRS` v1 layout. All integers little-endian, sections 8-byte aligned,
//! every byte not covered by the header, the section table or a section is 0.
//!
//! ```text
//! 0   magic "MWRULES\0"
//! 8   major u16, minor u16, flags u32 (0)
//! 16  section_count u32 (<= 64), reserved u32 (0)
//! 24  total_length u64 (== file length)
//! 32  counts 6 x u32: exact, suffix, keyword, ipv4 ranges, ipv6 ranges, subdomain
//! 56  reserved (0)
//! 64  section table: section_count x { kind u32, flags u32 (bit0 required), offset u64, length u64 }
//! ```

pub const MAJOR: u16 = 1;
pub const MINOR: u16 = 0;
/// Media type of the raw artifact bytes on the wire.
pub const MEDIA_TYPE: &str = "application/vnd.meshway.rules.v1";
pub const MAX_ARTIFACT_BYTES: usize = 16 << 20;
/// Exact + suffix + subdomain names per artifact.
pub const MAX_NAMES: usize = 1 << 20;
pub const MAX_KEYWORDS: usize = 1024;
/// Merged ranges per address family.
pub const MAX_RANGES: usize = 1 << 18;

pub(crate) const MAGIC: &[u8; 8] = b"MWRULES\0";
pub(crate) const HEADER: usize = 64;
pub(crate) const SECTION: usize = 24;
pub(crate) const MAX_SECTIONS: usize = 64;
pub(crate) const REQUIRED: u32 = 1;

pub(crate) const DOMAINS: u32 = 1;
pub(crate) const KEYWORDS: u32 = 2;
pub(crate) const IPV4: u32 = 3;
pub(crate) const IPV6: u32 = 4;

/// FST value bits; every key carries exactly one.
pub(crate) const EXACT: u64 = 1;
pub(crate) const SUFFIX: u64 = 2;
pub(crate) const SUBDOMAIN: u64 = 4;

/// Canonical member counts as stored in the header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Counts {
    pub exact: u32,
    pub suffix: u32,
    pub subdomain: u32,
    pub keyword: u32,
    pub ipv4: u32,
    pub ipv6: u32,
}

impl Counts {
    /// The design's `entries`: canonical names, keywords and merged ranges.
    pub fn entries(&self) -> u64 {
        [
            self.exact,
            self.suffix,
            self.subdomain,
            self.keyword,
            self.ipv4,
            self.ipv6,
        ]
        .iter()
        .map(|n| u64::from(*n))
        .sum()
    }
    pub(crate) fn names(&self) -> u64 {
        u64::from(self.exact) + u64::from(self.suffix) + u64::from(self.subdomain)
    }
    /// Header order.
    pub(crate) fn to_array(self) -> [u32; 6] {
        [
            self.exact,
            self.suffix,
            self.keyword,
            self.ipv4,
            self.ipv6,
            self.subdomain,
        ]
    }
    pub(crate) fn from_array(a: [u32; 6]) -> Self {
        Self {
            exact: a[0],
            suffix: a[1],
            keyword: a[2],
            ipv4: a[3],
            ipv6: a[4],
            subdomain: a[5],
        }
    }
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
pub(crate) fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}
pub(crate) fn u128_at(b: &[u8], at: usize) -> Option<u128> {
    Some(u128::from_le_bytes(
        b.get(at..at.checked_add(16)?)?.try_into().ok()?,
    ))
}

/// Writes header, table and aligned sections in the given order.
pub(crate) fn assemble(counts: Counts, sections: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = vec![0u8; HEADER + SECTION * sections.len()];
    let mut table = Vec::with_capacity(sections.len());
    for (kind, body) in sections {
        out.resize(out.len().next_multiple_of(8), 0);
        table.push((*kind, out.len() as u64, body.len() as u64));
        out.extend_from_slice(body);
    }
    out[0..8].copy_from_slice(MAGIC);
    out[8..10].copy_from_slice(&MAJOR.to_le_bytes());
    out[10..12].copy_from_slice(&MINOR.to_le_bytes());
    out[16..20].copy_from_slice(&(sections.len() as u32).to_le_bytes());
    let total = out.len() as u64;
    out[24..32].copy_from_slice(&total.to_le_bytes());
    for (i, n) in counts.to_array().iter().enumerate() {
        out[32 + i * 4..36 + i * 4].copy_from_slice(&n.to_le_bytes());
    }
    for (i, (kind, offset, length)) in table.iter().enumerate() {
        let at = HEADER + i * SECTION;
        out[at..at + 4].copy_from_slice(&kind.to_le_bytes());
        out[at + 4..at + 8].copy_from_slice(&REQUIRED.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
        out[at + 16..at + 24].copy_from_slice(&length.to_le_bytes());
    }
    out
}
