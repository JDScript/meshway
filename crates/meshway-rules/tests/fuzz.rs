//! Loader robustness: any byte string is either rejected or opens to a set
//! whose every query and iterator completes without panicking. Case counts
//! follow `PROPTEST_CASES`; long runs are driven by that variable.

use meshway_rules::{Builder, Cidr, Entry, Keyword, Name, RuleSet};
use proptest::prelude::*;
use std::net::IpAddr;

fn exercise(set: &RuleSet, queries: &[String], addrs: &[IpAddr]) {
    for q in queries {
        let _ = set.match_domain(q);
    }
    for a in addrs {
        let _ = set.match_ip(*a);
    }
    let names = set.domains().take(1 << 21).count() as u64;
    assert!(
        names
            <= set.counts().exact as u64
                + set.counts().suffix as u64
                + set.counts().subdomain as u64
    );
    assert_eq!(set.keywords().count(), set.counts().keyword as usize);
    assert_eq!(set.ipv4_ranges().count(), set.counts().ipv4 as usize);
    assert_eq!(set.ipv6_ranges().count(), set.counts().ipv6 as usize);
}

fn label() -> impl Strategy<Value = String> + Clone {
    "[a-z0-9]{1,6}"
}

fn entry() -> impl Strategy<Value = Entry> {
    let name =
        || prop::collection::vec(label(), 1..4).prop_map(|l| Name::parse(&l.join(".")).unwrap());
    prop_oneof![
        name().prop_map(Entry::Exact),
        name().prop_map(Entry::Suffix),
        name().prop_map(Entry::Subdomain),
        "[a-z]{2,5}".prop_map(|k| Entry::Keyword(Keyword::parse(&k).unwrap())),
        (any::<u32>(), 8u8..=32).prop_map(|(a, b)| Entry::Cidr(
            Cidr::parse(&format!("{}/{b}", std::net::Ipv4Addr::from(a))).unwrap()
        )),
        (any::<u128>(), 16u8..=128).prop_map(|(a, b)| Entry::Cidr(
            Cidr::parse(&format!("{}/{b}", std::net::Ipv6Addr::from(a))).unwrap()
        )),
    ]
}

fn queries() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop::collection::vec(label(), 1..5).prop_map(|l| l.join(".")),
        0..16,
    )
}

fn addrs() -> impl Strategy<Value = Vec<IpAddr>> {
    prop::collection::vec(
        prop_oneof![
            any::<u32>().prop_map(|a| IpAddr::from(std::net::Ipv4Addr::from(a))),
            any::<u128>().prop_map(|a| IpAddr::from(std::net::Ipv6Addr::from(a))),
        ],
        0..16,
    )
}

#[derive(Debug, Clone)]
enum Mutation {
    Set(usize, u8),
    Xor(usize, u8),
    Truncate(usize),
    Extend(Vec<u8>),
    /// Overwrite a header or section-table word, the fields most likely to
    /// steer the loader outside the buffer.
    Word(usize, u64),
}

fn mutation() -> impl Strategy<Value = Mutation> {
    prop_oneof![
        (any::<usize>(), any::<u8>()).prop_map(|(i, b)| Mutation::Set(i, b)),
        (any::<usize>(), 1u8..).prop_map(|(i, b)| Mutation::Xor(i, b)),
        any::<usize>().prop_map(Mutation::Truncate),
        prop::collection::vec(any::<u8>(), 1..64).prop_map(Mutation::Extend),
        (0usize..24, prop_oneof![any::<u64>(), 0u64..4096]).prop_map(|(w, v)| Mutation::Word(w, v)),
    ]
}

fn apply(bytes: &mut Vec<u8>, m: &Mutation) {
    let n = bytes.len();
    match m {
        Mutation::Set(i, b) if n > 0 => bytes[i % n] = *b,
        Mutation::Xor(i, b) if n > 0 => bytes[i % n] ^= *b,
        Mutation::Truncate(i) => bytes.truncate(i % (n + 1)),
        Mutation::Extend(tail) => bytes.extend(tail),
        Mutation::Word(w, v) => {
            // Words 0..8 are the header, 8.. the first section-table entries.
            let at = w * 8;
            if at + 8 <= n {
                bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
            }
        }
        _ => {}
    }
}

proptest! {
    #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..2048), q in queries(), a in addrs()) {
        if let Ok(set) = RuleSet::from_vec(bytes) {
            exercise(&set, &q, &a);
        }
    }

    #[test]
    fn mutated_artifacts_never_panic(
        entries in prop::collection::vec(entry(), 0..200),
        mutations in prop::collection::vec(mutation(), 1..6),
        fix_length in any::<bool>(),
        q in queries(),
        a in addrs(),
    ) {
        let mut builder = Builder::new();
        for e in entries {
            builder.insert(e);
        }
        let mut bytes = builder.build().unwrap().into_bytes();
        for m in &mutations {
            apply(&mut bytes, m);
        }
        if fix_length && bytes.len() >= 32 {
            // Keep `total_length` consistent so mutations reach the sections.
            let n = bytes.len() as u64;
            bytes[24..32].copy_from_slice(&n.to_le_bytes());
        }
        if let Ok(set) = RuleSet::from_vec(bytes) {
            exercise(&set, &q, &a);
        }
    }
}
