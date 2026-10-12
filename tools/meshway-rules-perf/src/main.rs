//! Open and match costs of real `MWRS` artifacts through the production
//! loader and matcher. Every timed query is first checked against an
//! independent naive matcher built from the artifact's own enumeration.
//! Prints one JSON object on stdout.
#![forbid(unsafe_code)]

use clap::Parser;
use meshway_rules::{Digest, DomainKind, RuleSet};
use serde::Serialize;
use std::{
    collections::HashMap,
    hint::black_box,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    time::Instant,
};

#[derive(Parser)]
struct Options {
    #[command(flatten)]
    logging: meshway_logging::Options,
    /// Timed queries per class.
    #[arg(long, default_value_t = 1_000_000)]
    queries: usize,
    /// Repeats per open measurement.
    #[arg(long, default_value_t = 200)]
    opens: usize,
    /// Rounds; the report keeps every round and the median.
    #[arg(long, default_value_t = 3)]
    samples: usize,
    artifacts: Vec<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    source_revision: String,
    host: String,
    queries: usize,
    opens: usize,
    samples: usize,
    sets: Vec<SetReport>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SetReport {
    file: String,
    digest: String,
    bytes: usize,
    entries: u64,
    keywords: u32,
    /// p50 µs per round: read + open (validation + digest) from the heap.
    read_open_heap_us: Vec<f64>,
    /// p50 µs per round: SHA-256 of the bytes alone (part of every open).
    digest_us: Vec<f64>,
    /// ns per query per round; null without members of that class.
    domain_hit_ns: Vec<Option<f64>>,
    domain_miss_ns: Vec<Option<f64>>,
    v4_hit_ns: Vec<Option<f64>>,
    v4_miss_ns: Vec<Option<f64>>,
    v6_hit_ns: Vec<Option<f64>>,
    v6_miss_ns: Vec<Option<f64>>,
    /// Production matcher against the naive reference, all queries.
    mismatches: usize,
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn label(&mut self) -> String {
        (0..3 + self.below(10))
            .map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[self.below(36)]))
            .collect()
    }
}

struct Naive {
    names: HashMap<String, DomainKind>,
    keywords: Vec<String>,
    v4: Vec<(u32, u32)>,
    v6: Vec<(u128, u128)>,
}

impl Naive {
    fn new(set: &RuleSet) -> Self {
        Self {
            names: set.domains().map(|d| (d.name, d.kind)).collect(),
            keywords: set.keywords().map(str::to_owned).collect(),
            v4: set
                .ipv4_ranges()
                .map(|r| (u32::from(*r.start()), u32::from(*r.end())))
                .collect(),
            v6: set
                .ipv6_ranges()
                .map(|r| (u128::from(*r.start()), u128::from(*r.end())))
                .collect(),
        }
    }
    fn domain(&self, q: &str) -> bool {
        let suffixes =
            std::iter::once((0, q)).chain(q.match_indices('.').map(|(i, _)| (i + 1, &q[i + 1..])));
        for (at, s) in suffixes {
            match self.names.get(s) {
                Some(DomainKind::Suffix) => return true,
                Some(DomainKind::Subdomain) if at > 0 => return true,
                Some(DomainKind::Exact) if at == 0 => return true,
                _ => {}
            }
        }
        self.keywords.iter().any(|k| q.contains(k.as_str()))
    }
    fn ip(&self, a: IpAddr) -> bool {
        match a {
            IpAddr::V4(a) => {
                let a = u32::from(a);
                let i = self.v4.partition_point(|r| r.0 <= a);
                i > 0 && self.v4[i - 1].1 >= a
            }
            IpAddr::V6(a) => {
                let a = u128::from(a);
                let i = self.v6.partition_point(|r| r.0 <= a);
                i > 0 && self.v6[i - 1].1 >= a
            }
        }
    }
}

fn p50(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn us(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e6
}

/// `None` when the set has no member of that class to query.
fn time_domain(set: &RuleSet, q: &[String]) -> Option<f64> {
    if q.is_empty() {
        return None;
    }
    let t = Instant::now();
    let mut n = 0usize;
    for name in q {
        n += usize::from(set.match_domain(black_box(name)).is_some());
    }
    black_box(n);
    Some(t.elapsed().as_secs_f64() * 1e9 / q.len() as f64)
}

fn time_ip(set: &RuleSet, q: &[IpAddr]) -> Option<f64> {
    if q.is_empty() {
        return None;
    }
    let t = Instant::now();
    let mut n = 0usize;
    for a in q {
        n += usize::from(set.match_ip(black_box(*a)));
    }
    black_box(n);
    Some(t.elapsed().as_secs_f64() * 1e9 / q.len() as f64)
}

fn measure(o: &Options, path: &PathBuf) -> SetReport {
    let bytes = std::fs::read(path).expect("artifact");
    let set = RuleSet::from_vec(bytes.clone()).expect("valid artifact");
    let naive = Naive::new(&set);

    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let domains: Vec<(String, DomainKind)> =
        naive.names.iter().map(|(n, k)| (n.clone(), *k)).collect();
    let mut hits = Vec::with_capacity(o.queries);
    if !domains.is_empty() || !naive.keywords.is_empty() {
        while hits.len() < o.queries {
            let q = if !naive.keywords.is_empty() && (domains.is_empty() || rng.below(10) == 0) {
                let k = &naive.keywords[rng.below(naive.keywords.len())];
                format!("{}{k}{}.com", rng.label(), rng.label())
            } else {
                let (n, kind) = &domains[rng.below(domains.len())];
                match kind {
                    DomainKind::Exact => n.clone(),
                    DomainKind::Subdomain => format!("{}.{n}", rng.label()),
                    DomainKind::Suffix if rng.below(2) == 0 => format!("{}.{n}", rng.label()),
                    DomainKind::Suffix => n.clone(),
                }
            };
            hits.push(q);
        }
    }
    let tlds = ["com", "net", "org", "io", "xyz", "dev", "app", "co.uk"];
    let mut misses = Vec::with_capacity(o.queries);
    while misses.len() < o.queries && !(domains.is_empty() && naive.keywords.is_empty()) {
        let q = format!(
            "{}.{}.{}",
            rng.label(),
            rng.label(),
            tlds[rng.below(tlds.len())]
        );
        if !naive.domain(&q) {
            misses.push(q);
        }
    }
    let ip_queries = |rng: &mut Rng, v6: bool, hit: bool| -> Vec<IpAddr> {
        let mut out = vec![];
        let ranges: Vec<(u128, u128)> = if v6 {
            naive.v6.clone()
        } else {
            naive
                .v4
                .iter()
                .map(|(s, e)| (u128::from(*s), u128::from(*e)))
                .collect()
        };
        if ranges.is_empty() {
            return out;
        }
        let make = |v: u128| {
            if v6 {
                IpAddr::V6(Ipv6Addr::from(v))
            } else {
                IpAddr::V4(Ipv4Addr::from(v as u32))
            }
        };
        while out.len() < o.queries {
            let v = if hit {
                let (s, e) = ranges[rng.below(ranges.len())];
                let span = e - s;
                let r = (u128::from(rng.next()) << 64) | u128::from(rng.next());
                s + if span == u128::MAX { r } else { r % (span + 1) }
            } else if v6 {
                (0x2000u128 << 112) | (u128::from(rng.next()) << 48) | u128::from(rng.next() >> 16)
            } else {
                u128::from(rng.next() as u32)
            };
            let a = make(v);
            if naive.ip(a) == hit {
                out.push(a);
            }
        }
        out
    };
    let (v4h, v4m) = (
        ip_queries(&mut rng, false, true),
        ip_queries(&mut rng, false, false),
    );
    let (v6h, v6m) = (
        ip_queries(&mut rng, true, true),
        ip_queries(&mut rng, true, false),
    );

    let mut report = SetReport {
        file: path.display().to_string(),
        digest: set.digest().to_string(),
        bytes: bytes.len(),
        entries: set.counts().entries(),
        keywords: set.counts().keyword,
        ..SetReport::default()
    };
    report.mismatches = hits
        .iter()
        .chain(&misses)
        .filter(|q| set.match_domain(q).is_some() != naive.domain(q))
        .count()
        + v4h
            .iter()
            .chain(&v4m)
            .chain(&v6h)
            .chain(&v6m)
            .filter(|a| set.match_ip(**a) != naive.ip(**a))
            .count();

    for _ in 0..o.samples {
        let (mut heap, mut sha) = (vec![], vec![]);
        for _ in 0..o.opens {
            let t = Instant::now();
            black_box(RuleSet::from_vec(std::fs::read(path).expect("read")).expect("open"));
            heap.push(us(t));
            let t = Instant::now();
            black_box(Digest::of(black_box(&bytes)));
            sha.push(us(t));
        }
        report.read_open_heap_us.push(p50(heap));
        report.digest_us.push(p50(sha));
        report.domain_hit_ns.push(time_domain(&set, &hits));
        report.domain_miss_ns.push(time_domain(&set, &misses));
        report.v4_hit_ns.push(time_ip(&set, &v4h));
        report.v4_miss_ns.push(time_ip(&set, &v4m));
        report.v6_hit_ns.push(time_ip(&set, &v6h));
        report.v6_miss_ns.push(time_ip(&set, &v6m));
    }
    report
}

fn command(program: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .expect("command runs");
    assert!(out.status.success(), "{program} failed");
    String::from_utf8(out.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

fn main() {
    let o = Options::parse();
    o.logging.init().expect("logging");
    let report = Report {
        source_revision: command("git", &["rev-parse", "HEAD"]),
        host: format!("{} {}", command("uname", &["-srm"]), std::env::consts::ARCH),
        queries: o.queries,
        opens: o.opens,
        samples: o.samples,
        sets: o.artifacts.iter().map(|p| measure(&o, p)).collect(),
    };
    println!("{}", serde_json::to_string_pretty(&report).expect("json"));
}
