//! Artifact layout, minimization, determinism and loader rejection.

use meshway_rules::{
    Builder, Cidr, Counts, DomainEntry, DomainHit, DomainKind, Entry, Keyword, LoadError, Name,
    RuleSet,
};
use std::net::IpAddr;

fn name(n: &str) -> Name {
    Name::parse(n).unwrap()
}

fn build(entries: &[Entry]) -> meshway_rules::Artifact {
    let mut b = Builder::new();
    for e in entries {
        b.insert(e.clone());
    }
    b.build().unwrap()
}

fn sample() -> Vec<Entry> {
    vec![
        Entry::Exact(name("exact.example")),
        Entry::Suffix(name("suffix.example")),
        Entry::Subdomain(name("sub.example")),
        Entry::Keyword(Keyword::parse("track").unwrap()),
        Entry::Cidr(Cidr::parse("192.0.2.0/24").unwrap()),
        Entry::Cidr(Cidr::parse("2001:db8::/32").unwrap()),
    ]
}

#[test]
fn matching_semantics() {
    let s = build(&sample()).open().unwrap();
    let cases = [
        ("exact.example", Some(DomainHit::Exact)),
        ("a.exact.example", None),
        ("suffix.example", Some(DomainHit::Suffix { len: 14 })),
        ("a.b.suffix.example", Some(DomainHit::Suffix { len: 14 })),
        ("notsuffix.example", None),
        ("sub.example", None),
        ("a.sub.example", Some(DomainHit::Subdomain { len: 11 })),
        ("xsub.example", None),
        ("bigtracker.example", Some(DomainHit::Keyword { index: 0 })),
        ("example", None),
        ("", None),
    ];
    for (query, hit) in cases {
        assert_eq!(s.match_domain(query), hit, "{query}");
    }
    assert!(s.match_ip("192.0.2.255".parse().unwrap()));
    assert!(!s.match_ip("192.0.3.0".parse().unwrap()));
    assert!(s.match_ip("2001:db8:1::".parse().unwrap()));
    assert!(!s.match_ip("2001:db9::".parse().unwrap()));
    // An IPv4 query never consults IPv6 ranges, including mapped forms.
    assert!(!s.match_ip("::ffff:192.0.2.1".parse().unwrap()));
}

#[test]
fn full_address_space_edges() {
    let s = build(&[
        Entry::Cidr(Cidr::parse("0.0.0.0/1").unwrap()),
        Entry::Cidr(Cidr::parse("128.0.0.0/1").unwrap()),
        Entry::Cidr(Cidr::parse("ffff::/16").unwrap()),
        Entry::Cidr(Cidr::parse("::/128").unwrap()),
    ])
    .open()
    .unwrap();
    assert_eq!(s.counts().ipv4, 1);
    assert_eq!(s.counts().ipv6, 2);
    for a in [
        "0.0.0.0",
        "255.255.255.255",
        "::",
        "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
    ] {
        assert!(s.match_ip(a.parse::<IpAddr>().unwrap()), "{a}");
    }
    assert!(!s.match_ip("::1".parse().unwrap()));
    let v6: Vec<String> = s
        .ipv6_ranges()
        .map(|r| format!("{}-{}", r.start(), r.end()))
        .collect();
    assert_eq!(
        v6,
        ["::-::", "ffff::-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"]
    );
}

#[test]
fn minimization_keeps_only_uncovered_members() {
    let a = build(&[
        // exact + subdomain of the same name is a suffix
        Entry::Exact(name("both.example")),
        Entry::Subdomain(name("both.example")),
        Entry::Exact(name("x.both.example")),
        // a subdomain member covers everything strictly below it
        Entry::Subdomain(name("cdn.example")),
        Entry::Exact(name("a.cdn.example")),
        Entry::Suffix(name("b.cdn.example")),
        Entry::Subdomain(name("c.cdn.example")),
        Entry::Exact(name("cdn.example")),
        // keyword coverage
        Entry::Keyword(Keyword::parse("ads").unwrap()),
        Entry::Keyword(Keyword::parse("myads").unwrap()),
        Entry::Suffix(name("ads.example")),
        Entry::Exact(name("kept.example")),
        Entry::Exact(name("kept.example")),
    ]);
    let s = a.clone().open().unwrap();
    let domains: Vec<DomainEntry> = s.domains().collect();
    let expected = [
        (DomainKind::Suffix, "cdn.example"),
        (DomainKind::Exact, "kept.example"),
        (DomainKind::Suffix, "both.example"),
    ];
    let mut domains: Vec<(DomainKind, &str)> =
        domains.iter().map(|d| (d.kind, d.name.as_str())).collect();
    domains.sort_by_key(|d| d.1);
    let mut expected = expected.to_vec();
    expected.sort_by_key(|d| d.1);
    assert_eq!(domains, expected);
    assert_eq!(s.keywords().collect::<Vec<_>>(), ["ads"]);
    assert_eq!(a.redundant(), 13 - 4);
}

#[test]
fn equal_sets_give_equal_bytes() {
    let mut entries = sample();
    let first = build(&entries);
    entries.reverse();
    entries.extend(sample());
    let second = build(&entries);
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.digest(), second.digest());
    assert_eq!(
        first.digest(),
        RuleSet::from_vec(first.bytes().to_vec()).unwrap().digest()
    );
    let empty = Builder::new().build().unwrap();
    let s = empty.open().unwrap();
    assert_eq!(s.counts(), Counts::default());
    assert_eq!(s.match_domain("a.example"), None);
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

/// Table entry `i` field offsets: kind +0, flags +4, offset +8, length +16.
fn entry(i: usize) -> usize {
    64 + 24 * i
}

fn rejected(bytes: Vec<u8>) -> LoadError {
    RuleSet::from_vec(bytes).expect_err("must be rejected")
}

#[test]
fn loader_rejects_malformed_layouts() {
    let good = build(&sample()).into_bytes();
    let n = good.len();
    let invalid = |reason: &'static str| LoadError::Invalid(reason);
    let mutate = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut b = good.clone();
        f(&mut b);
        rejected(b)
    };
    assert_eq!(mutate(&|b| b[0] = b'X'), invalid("magic"));
    assert_eq!(
        mutate(&|b| put(b, 8, &2u16.to_le_bytes())),
        LoadError::UnsupportedMajor(2)
    );
    assert_eq!(mutate(&|b| b[12] = 1), invalid("reserved header bytes"));
    assert_eq!(mutate(&|b| b[60] = 1), invalid("reserved header bytes"));
    assert_eq!(mutate(&|b| b.truncate(n - 1)), invalid("total length"));
    assert_eq!(
        mutate(&|b| {
            b.push(1);
            put(b, 24, &((n + 1) as u64).to_le_bytes());
        }),
        invalid("nonzero padding")
    );
    assert_eq!(
        mutate(&|b| put(b, 16, &65u32.to_le_bytes())),
        invalid("section table")
    );
    // Offsets past the end, oversized lengths, misalignment, overlap.
    assert_eq!(
        mutate(&|b| put(b, entry(2) + 8, &(n as u64 + 8).to_le_bytes())),
        invalid("section bounds")
    );
    assert_eq!(
        mutate(&|b| put(b, entry(2) + 16, &u64::MAX.to_le_bytes())),
        invalid("section bounds")
    );
    assert_eq!(
        mutate(&|b| {
            let off = u64::from_le_bytes(b[entry(1) + 8..entry(1) + 16].try_into().unwrap());
            put(b, entry(1) + 8, &(off + 4).to_le_bytes());
        }),
        invalid("section bounds")
    );
    assert_eq!(
        mutate(&|b| {
            let domains = b[entry(0) + 8..entry(0) + 16].to_vec();
            put(b, entry(1) + 8, &domains);
        }),
        invalid("overlapping sections")
    );
    assert_eq!(
        mutate(&|b| put(b, entry(0) + 8, &0u64.to_le_bytes())),
        invalid("section bounds")
    );
    // Section kinds and flags.
    assert_eq!(
        mutate(&|b| put(b, entry(3), &3u32.to_le_bytes())),
        invalid("duplicate section")
    );
    assert_eq!(
        mutate(&|b| put(b, entry(3), &99u32.to_le_bytes())),
        invalid("unknown required section")
    );
    assert_eq!(
        mutate(&|b| {
            put(b, entry(3), &99u32.to_le_bytes());
            put(b, entry(3) + 4, &0u32.to_le_bytes());
        }),
        invalid("missing section")
    );
    assert_eq!(mutate(&|b| b[entry(0) + 4] = 3), invalid("section flags"));
    // Header counts must describe the sections.
    assert_eq!(
        mutate(&|b| put(b, 32, &2u32.to_le_bytes())),
        invalid("domain count")
    );
    assert_eq!(
        mutate(&|b| put(b, 40, &2u32.to_le_bytes())),
        invalid("keyword count")
    );
    assert_eq!(
        mutate(&|b| put(b, 44, &2u32.to_le_bytes())),
        invalid("range count")
    );
    assert_eq!(
        mutate(&|b| put(b, 32, &u32::MAX.to_le_bytes())),
        invalid("counts exceed limits")
    );
    assert_eq!(rejected(vec![0; (16 << 20) + 1]), LoadError::TooLarge);
}

#[test]
fn loader_rejects_unordered_ranges_and_keywords() {
    let good = build(&[
        Entry::Cidr(Cidr::parse("10.0.0.0/8").unwrap()),
        Entry::Cidr(Cidr::parse("12.0.0.0/8").unwrap()),
        Entry::Keyword(Keyword::parse("aa").unwrap()),
        Entry::Keyword(Keyword::parse("bb").unwrap()),
    ])
    .into_bytes();
    let section = |b: &[u8], i: usize| {
        u64::from_le_bytes(b[entry(i) + 8..entry(i) + 16].try_into().unwrap()) as usize
    };
    // Adjacent ranges must have been merged: 10/8 then 11/8 is not canonical.
    let mut b = good.clone();
    let v4 = section(&b, 2);
    put(
        &mut b,
        v4 + 8,
        &u32::from_be_bytes([11, 0, 0, 0]).to_le_bytes(),
    );
    put(
        &mut b,
        v4 + 12,
        &u32::from_be_bytes([11, 255, 255, 255]).to_le_bytes(),
    );
    assert_eq!(rejected(b), LoadError::Invalid("range order"));
    let mut b = good.clone();
    put(&mut b, v4, &u32::MAX.to_le_bytes());
    assert_eq!(rejected(b), LoadError::Invalid("range order"));
    // Keyword blob "aabb": swapping to "bbaa" breaks the order.
    let mut b = good.clone();
    let kw = section(&b, 1);
    put(&mut b, kw + 16, b"bbaa");
    assert_eq!(rejected(b), LoadError::Invalid("keyword order"));
    let mut b = good;
    put(&mut b, kw + 16, b"A");
    assert_eq!(rejected(b), LoadError::Invalid("keyword table"));
}
