//! R0 rule-set spike. Commands (paths relative to the spike root, usually
//! `<repo>/.meshway/rules-spike`):
//!
//! - `coverage <root>`: convert every source, write `<root>/out/*.mwrs` and
//!   `<root>/out/coverage.json`.
//! - `bench <root> <set> <iters>`: load and match timings for one artifact.
//! - `memory <root> <set> <handles> <heap|mmap|shared>`: resident memory of
//!   many independently opened handles.
//! - `r1diff <root> <random>`: production `meshway-rules` against the spike
//!   reference on every input, writing `<root>/out-r1/*.mwrs`.

mod format;
mod r1diff;
mod source;

use format::{Backing, Canonical, Reference, RuleSet};
use serde::Serialize;
use source::Format;
use std::hint::black_box;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// (set name, file, spike format, current Server format if any)
const SOURCES: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "sing-geosite-cn",
        "data/geosite-cn.json",
        "sing-box",
        Some("sing-box-source"),
    ),
    (
        "sing-geosite-openai",
        "data/geosite-openai.json",
        "sing-box",
        Some("sing-box-source"),
    ),
    (
        "sing-geosite-geolocation-!cn",
        "data/geosite-geolocation-!cn.json",
        "sing-box",
        Some("sing-box-source"),
    ),
    (
        "sing-geoip-cn",
        "data/geoip-cn.json",
        "sing-box",
        Some("sing-box-source"),
    ),
    (
        "mihomo-geosite-cn",
        "data/mihomo-geosite-cn.yaml",
        "mihomo-domain",
        Some("mihomo-domain"),
    ),
    (
        "mihomo-geosite-openai",
        "data/mihomo-geosite-openai.yaml",
        "mihomo-domain",
        Some("mihomo-domain"),
    ),
    (
        "mihomo-geosite-geolocation-!cn",
        "data/mihomo-geosite-geolocation-!cn.yaml",
        "mihomo-domain",
        Some("mihomo-domain"),
    ),
    (
        "mihomo-geoip-cn",
        "data/mihomo-geoip-cn.yaml",
        "mihomo-ipcidr",
        Some("mihomo-ipcidr"),
    ),
    (
        "mihomo-geosite-cn-text",
        "data/mihomo-geosite-cn.list",
        "mihomo-domain-text",
        None,
    ),
    (
        "mihomo-geosite-openai-text",
        "data/mihomo-geosite-openai.list",
        "mihomo-domain-text",
        None,
    ),
    (
        "mihomo-geosite-geolocation-!cn-text",
        "data/mihomo-geosite-geolocation-!cn.list",
        "mihomo-domain-text",
        None,
    ),
    (
        "mihomo-geoip-cn-text",
        "data/mihomo-geoip-cn.list",
        "mihomo-ipcidr-text",
        None,
    ),
    ("bm7-global", "extra/bm7-Global.list", "classical", None),
    ("bm7-china", "extra/bm7-China.list", "classical", None),
    ("bm7-openai", "extra/bm7-OpenAI.list", "classical", None),
    (
        "bm7-advertising",
        "extra/bm7-Advertising.list",
        "classical",
        None,
    ),
];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(2).expect("root or count"));
    match args.get(1).map(String::as_str) {
        Some("coverage") => coverage(&root),
        Some("bench") => bench(&root, &args[3], args[4].parse().expect("iters")),
        Some("memory") => memory(
            &root,
            &args[3],
            args[4].parse().expect("handles"),
            &args[5],
            args.get(6).map_or("mwrs", String::as_str),
        ),
        Some("acbuild") => acbuild(args[2].parse().expect("keywords")),
        Some("r1diff") => r1diff::run(&root, SOURCES, args[3].parse().expect("random queries")),
        _ => panic!("usage: coverage|bench|memory|acbuild"),
    }
}

fn find(name: &str) -> (&'static str, Format, Option<&'static str>) {
    let (_, file, format, server) = SOURCES.iter().find(|s| s.0 == name).expect("known set");
    (file, Format::parse(format).expect("format"), *server)
}

fn load_canonical(root: &Path, name: &str) -> (Vec<u8>, source::Parsed, Canonical) {
    let (file, format, _) = find(name);
    let bytes = std::fs::read(root.join(file)).expect("source");
    let parsed = source::parse(&bytes, format).expect("parse");
    let canonical = format::canonicalize(&parsed);
    (bytes, parsed, canonical)
}

#[derive(Serialize)]
struct Coverage {
    set: String,
    file: String,
    source_bytes: usize,
    /// Real `meshway_server::resources::convert` outcome.
    current_converter: String,
    current_entries: Option<usize>,
    current_json_bytes: Option<usize>,
    report: source::Report,
    /// exact, suffix, keyword, v4 ranges, v6 ranges, subdomain
    canonical: [usize; 6],
    redundant: usize,
    v4_min_prefixes: usize,
    artifact_bytes: usize,
    artifact_digest: String,
    build_ms: f64,
}

#[cfg(feature = "server")]
fn current(bytes: &[u8], format: Option<&str>) -> (String, Option<usize>, Option<usize>) {
    use meshway_configuration::RuleSetFormat;
    let Some(format) = format else {
        return ("unsupported-format".into(), None, None);
    };
    let format: RuleSetFormat =
        serde_json::from_value(serde_json::Value::String(format.into())).expect("format");
    match meshway_server::resources::convert(bytes, &format) {
        Ok(data) => (
            "ok".into(),
            Some(meshway_server::resources::entries(&data)),
            Some(serde_json::to_vec(&data).expect("json").len()),
        ),
        Err(problem) => (
            format!(
                "failed:{}",
                serde_json::to_string(&problem).expect("problem")
            ),
            None,
            None,
        ),
    }
}
#[cfg(not(feature = "server"))]
fn current(_: &[u8], _: Option<&str>) -> (String, Option<usize>, Option<usize>) {
    ("not-measured".into(), None, None)
}

fn coverage(root: &Path) {
    let out = root.join("out");
    std::fs::create_dir_all(&out).expect("out dir");
    let mut rows = vec![];
    for (name, file, _, server) in SOURCES {
        let (bytes, parsed, c) = load_canonical(root, name);
        let (current_converter, current_entries, current_json_bytes) = current(&bytes, *server);
        let start = Instant::now();
        let artifact = format::build(&c);
        let build_ms = ms(start.elapsed());
        RuleSet::open(Arc::new(artifact.clone()))
            .expect("open")
            .verify()
            .expect("verify");
        std::fs::write(out.join(format!("{name}.mwrs")), &artifact).expect("write");
        let row = Coverage {
            set: name.to_string(),
            file: file.to_string(),
            source_bytes: bytes.len(),
            current_converter,
            current_entries,
            current_json_bytes,
            canonical: [
                c.exact.len(),
                c.suffix.len(),
                c.keyword.len(),
                c.v4.len(),
                c.v6.len(),
                c.subdomain.len(),
            ],
            redundant: c.redundant,
            v4_min_prefixes: format::v4_prefix_count(&c.v4),
            artifact_bytes: artifact.len(),
            artifact_digest: format!("sha256:{}", format::digest(&artifact)),
            build_ms,
            report: parsed.report,
        };
        println!("{}", serde_json::to_string(&row).expect("json"));
        rows.push(row);
    }
    std::fs::write(
        out.join("coverage.json"),
        serde_json::to_vec_pretty(&rows).expect("json"),
    )
    .expect("write");
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
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
        let len = 3 + self.below(10);
        (0..len)
            .map(|_| b"abcdefghijklmnopqrstuvwxyz0123456789"[self.below(36)] as char)
            .collect()
    }
}

fn percentile(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn mmap(path: &Path) -> Backing {
    let file = std::fs::File::open(path).expect("artifact");
    // SAFETY: spike-only; the artifact file is private and never modified.
    Arc::new(unsafe { memmap2::Mmap::map(&file) }.expect("mmap"))
}

fn per_op(d: Duration, n: usize) -> f64 {
    d.as_secs_f64() * 1e9 / n as f64
}

fn bench(root: &Path, name: &str, iters: usize) {
    let (_, _, c) = load_canonical(root, name);
    let path = root.join("out").join(format!("{name}.mwrs"));
    let size = std::fs::metadata(&path).expect("artifact").len();
    println!(
        "set={name} artifact_bytes={size} canonical(exact,suffix,keyword,v4,v6,subdomain)={:?}",
        [
            c.exact.len(),
            c.suffix.len(),
            c.keyword.len(),
            c.v4.len(),
            c.v6.len(),
            c.subdomain.len()
        ]
    );

    // Load: first open in this process, then 1000 repeats.
    let t = Instant::now();
    let first = RuleSet::open(mmap(&path)).expect("open");
    println!(
        "load first_mmap_open_us={:.1}",
        t.elapsed().as_secs_f64() * 1e6
    );
    black_box(&first);
    let mut mm = vec![];
    let mut heap = vec![];
    let mut sha = vec![];
    let mut verify = vec![];
    for _ in 0..1000 {
        let t = Instant::now();
        black_box(RuleSet::open(mmap(&path)).expect("open"));
        mm.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        let bytes = std::fs::read(&path).expect("read");
        let set = RuleSet::open(Arc::new(bytes)).expect("open");
        heap.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        let backing = mmap(&path);
        black_box(format::digest((*backing).as_ref()));
        sha.push(t.elapsed().as_secs_f64() * 1e6);
        let t = Instant::now();
        set.verify().expect("verify");
        verify.push(t.elapsed().as_secs_f64() * 1e6);
    }
    for (label, v) in [
        ("mmap_open", mm),
        ("read_heap_open", heap),
        ("mmap_sha256", sha),
        ("verify", verify),
    ] {
        println!(
            "load {label}_us p50={:.1} p99={:.1}",
            percentile(v.clone(), 0.5),
            percentile(v, 0.99)
        );
    }
    // Baselines: what loading costs when the content is deserialised.
    let t = Instant::now();
    let mut exact_set = std::collections::HashSet::new();
    let mut set = std::collections::HashSet::new();
    for e in &c.exact {
        exact_set.insert(e.clone());
    }
    for e in &c.suffix {
        set.insert(e.clone());
    }
    println!("load baseline_hashset_build_ms={:.2}", ms(t.elapsed()));
    current_json_load(root, name);

    let hashed_bytes = format::build_with(&c, true);
    let hashed_path = root.join("out").join(format!("{name}.mwrh"));
    std::fs::write(&hashed_path, &hashed_bytes).expect("write");
    let mut hopen = vec![];
    for _ in 0..1000 {
        let t = Instant::now();
        black_box(RuleSet::open(mmap(&hashed_path)).expect("open"));
        hopen.push(t.elapsed().as_secs_f64() * 1e6);
    }
    println!(
        "variant fst+hash_index artifact_bytes={} mmap_open_us p50={:.1}",
        hashed_bytes.len(),
        percentile(hopen, 0.5)
    );
    let hm = RuleSet::open(mmap(&hashed_path)).expect("open");
    let reference = Reference { c: &c };
    let m = first;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let names: Vec<&String> = c.exact.iter().chain(&c.suffix).collect();
    let subs: Vec<&String> = c.subdomain.iter().collect();
    if !names.is_empty() || !c.keyword.is_empty() {
        let keywords: Vec<&String> = c.keyword.iter().collect();
        let hits: Vec<String> = (0..iters)
            .map(|_| {
                if !subs.is_empty() && rng.below(50) == 0 {
                    format!("{}.{}", rng.label(), subs[rng.below(subs.len())])
                } else if !keywords.is_empty() && (names.is_empty() || rng.below(10) == 0) {
                    format!(
                        "{}{}{}.com",
                        rng.label(),
                        keywords[rng.below(keywords.len())],
                        rng.label()
                    )
                } else {
                    let i = rng.below(names.len());
                    let n = names[i];
                    if i >= c.exact.len() && rng.below(2) == 0 {
                        format!("{}.{n}", rng.label())
                    } else {
                        n.to_string()
                    }
                }
            })
            .collect();
        let tlds = ["com", "net", "org", "io", "xyz", "dev", "app", "co.uk"];
        let misses: Vec<String> = (0..iters)
            .map(|_| {
                loop {
                    let q = format!(
                        "{}.{}.{}",
                        rng.label(),
                        rng.label(),
                        tlds[rng.below(tlds.len())]
                    );
                    if !reference.domain(&q) {
                        break q;
                    }
                }
            })
            .collect();
        for (label, queries, expect) in [("hit", &hits, true), ("miss", &misses, false)] {
            let mismatches = queries
                .iter()
                .filter(|q| m.match_domain(q.as_bytes()).is_some() != reference.domain(q))
                .count();
            let wrong = queries
                .iter()
                .filter(|q| reference.domain(q) != expect)
                .count();
            let t = Instant::now();
            let mut count = 0usize;
            for q in queries.iter() {
                count += usize::from(m.match_domain(black_box(q.as_bytes())).is_some());
            }
            let fst_ns = per_op(t.elapsed(), queries.len());
            let t = Instant::now();
            let mut hcount = 0usize;
            for q in queries.iter() {
                let q = black_box(q.as_str());
                let suffix_hit = exact_set.contains(q)
                    || std::iter::once(q)
                        .chain(q.match_indices('.').map(|(i, _)| &q[i + 1..]))
                        .any(|s| set.contains(s));
                hcount += usize::from(suffix_hit);
            }
            let hash_ns = per_op(t.elapsed(), queries.len());
            let hmis = queries
                .iter()
                .filter(|q| hm.match_domain_hashed(q.as_bytes()).is_some() != reference.domain(q))
                .count();
            let t = Instant::now();
            let mut hc = 0usize;
            for q in queries.iter() {
                hc += usize::from(hm.match_domain_hashed(black_box(q.as_bytes())).is_some());
            }
            println!(
                "  mwrs_hash_index {label} matched={hc} mismatches_vs_reference={hmis} ns={:.1}",
                per_op(t.elapsed(), queries.len())
            );
            println!(
                "match_domain {label} n={} matched={count} mismatches_vs_reference={mismatches} wrong_labels={wrong} mwrs_ns={fst_ns:.1} baseline_hashset_ns={hash_ns:.1} (hashset_matched={hcount})",
                queries.len()
            );
            current_policy_match(&c, queries);
        }
    }
    for family in [4u8, 6] {
        let ranges: Vec<(u128, u128)> = if family == 4 {
            c.v4.iter()
                .map(|(s, e)| (u128::from(*s), u128::from(*e)))
                .collect()
        } else {
            c.v6.clone()
        };
        if ranges.is_empty() {
            continue;
        }
        let addr = |v: u128| -> IpAddr {
            if family == 4 {
                IpAddr::V4(Ipv4Addr::from(v as u32))
            } else {
                IpAddr::V6(Ipv6Addr::from(v))
            }
        };
        let contains = |v: u128| {
            let i = ranges.partition_point(|r| r.0 <= v);
            i > 0 && ranges[i - 1].1 >= v
        };
        let wide = |rng: &mut Rng| -> u128 {
            if family == 4 {
                u128::from(rng.next() as u32)
            } else {
                // Global unicast space keeps misses realistic for IPv6.
                (0x2000u128 << 112) | (u128::from(rng.next()) << 48) | u128::from(rng.next() >> 16)
            }
        };
        let hits: Vec<IpAddr> = (0..iters)
            .map(|_| {
                let (s, e) = ranges[rng.below(ranges.len())];
                let span = e - s;
                let off = if span == u128::MAX {
                    0
                } else {
                    (u128::from(rng.next()) << 64 | u128::from(rng.next())) % (span + 1)
                };
                addr(s + off)
            })
            .collect();
        let misses: Vec<IpAddr> = (0..iters)
            .map(|_| {
                loop {
                    let v = wide(&mut rng);
                    if !contains(v) {
                        break addr(v);
                    }
                }
            })
            .collect();
        for (label, queries) in [("hit", &hits), ("miss", &misses)] {
            let sample_mismatch = queries
                .iter()
                .take(2000)
                .filter(|a| m.match_ip(**a) != reference.ip(**a))
                .count();
            let all_mismatch = queries
                .iter()
                .filter(|a| {
                    let v = match a {
                        IpAddr::V4(x) => u128::from(u32::from(*x)),
                        IpAddr::V6(x) => u128::from(*x),
                    };
                    m.match_ip(**a) != contains(v)
                })
                .count();
            let t = Instant::now();
            let mut count = 0usize;
            for a in queries.iter() {
                count += usize::from(m.match_ip(black_box(*a)));
            }
            println!(
                "match_ip v{family} {label} n={} matched={count} mismatches_vs_linear_2000={sample_mismatch} mismatches_vs_partition_point={all_mismatch} ns={:.1}",
                queries.len(),
                per_op(t.elapsed(), queries.len())
            );
        }
    }
}

#[cfg(feature = "server")]
fn current_policy_match(c: &Canonical, queries: &[String]) {
    // The current Policy `plan_match` path: linear scan with per-entry
    // normalisation over exact then suffix lists.
    let exact: Vec<&String> = c.exact.iter().collect();
    let suffix: Vec<&String> = c.suffix.iter().collect();
    let n = queries.len().min(200);
    let t = Instant::now();
    let mut count = 0usize;
    for q in &queries[..n] {
        let hit = exact
            .iter()
            .any(|d| meshway_policy::domain_name_matches(q, d))
            || suffix
                .iter()
                .any(|s| meshway_policy::domain_suffix_matches(q, s));
        count += usize::from(black_box(hit));
    }
    println!(
        "  current_policy_linear n={n} matched={count} ns={:.0}",
        per_op(t.elapsed(), n)
    );
}
#[cfg(not(feature = "server"))]
fn current_policy_match(_: &Canonical, _: &[String]) {}

#[cfg(feature = "server")]
fn current_json_load(root: &Path, name: &str) {
    let (file, _, server) = find(name);
    let Some(format) = server else { return };
    let bytes = std::fs::read(root.join(file)).expect("source");
    let format: meshway_configuration::RuleSetFormat =
        serde_json::from_value(serde_json::Value::String(format.into())).expect("format");
    let Ok(data) = meshway_server::resources::convert(&bytes, &format) else {
        println!("load current_json: converter fails, not measurable");
        return;
    };
    let json = serde_json::to_vec(&data).expect("json");
    let mut v = vec![];
    for _ in 0..50 {
        let t = Instant::now();
        let parsed: meshway_model::resources::RuleSetData =
            serde_json::from_slice(&json).expect("json");
        black_box(parsed);
        v.push(ms(t.elapsed()));
    }
    println!(
        "load current_json_bytes={} current_json_parse_ms p50={:.2}",
        json.len(),
        percentile(v, 0.5)
    );
}
#[cfg(not(feature = "server"))]
fn current_json_load(_: &Path, _: &str) {}

fn memory(root: &Path, name: &str, handles: usize, mode: &str, ext: &str) {
    let path = root.join("out").join(format!("{name}.{ext}"));
    let (_, _, c) = load_canonical(root, name);
    let probes: Vec<String> = c
        .suffix
        .iter()
        .chain(&c.exact)
        .step_by(97)
        .cloned()
        .collect();
    let before = rss();
    let shared = mmap(&path);
    let sets: Vec<RuleSet> = (0..handles)
        .map(|_| {
            let backing = match mode {
                "heap" => Arc::new(std::fs::read(&path).expect("read")) as Backing,
                "mmap" => mmap(&path),
                "shared" => shared.clone(),
                _ => panic!("mode"),
            };
            RuleSet::open(backing).expect("open")
        })
        .collect();
    // Touch every handle with a spread of queries so the pages are resident.
    let mut hits = 0usize;
    for set in &sets {
        for p in &probes {
            hits += usize::from(if ext == "mwrh" {
                set.match_domain_hashed(p.as_bytes()).is_some()
            } else {
                set.match_domain(p.as_bytes()).is_some()
            });
        }
    }
    let after = rss();
    println!(
        "memory set={name} variant={ext} mode={mode} handles={handles} probes_per_handle={} hits={hits} artifact_bytes={}\n  before: {before}\n  after:  {after}",
        probes.len(),
        std::fs::metadata(&path).expect("artifact").len()
    );
    black_box(&sets);
}

fn rss() -> String {
    if let Ok(s) = std::fs::read_to_string("/proc/self/smaps_rollup") {
        return s
            .lines()
            .filter(|l| {
                [
                    "Rss:",
                    "Pss:",
                    "Shared_Clean:",
                    "Private_Clean:",
                    "Private_Dirty:",
                ]
                .iter()
                .any(|k| l.starts_with(k))
            })
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let pid = std::process::id().to_string();
    let ps = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let footprint = std::process::Command::new("footprint")
        .arg(&pid)
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .find(|l| l.contains("Footprint:"))
                .unwrap_or("")
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    format!("ps_rss_kb={ps} {footprint}")
}

/// Synthetic keyword scale (random 5-12 byte keywords): automaton build time,
/// heap and match cost. Real lists stay in `bench`.
fn acbuild(n: usize) {
    use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let words: Vec<String> = (0..n).map(|_| rng.label()).collect();
    for kind in [AhoCorasickKind::DFA, AhoCorasickKind::ContiguousNFA] {
        let mut times = vec![];
        let mut ac = None;
        for _ in 0..5 {
            let t = Instant::now();
            ac = Some(
                AhoCorasick::builder()
                    .kind(Some(kind))
                    .match_kind(MatchKind::LeftmostFirst)
                    .build(&words)
                    .expect("build"),
            );
            times.push(ms(t.elapsed()));
        }
        let ac = ac.expect("built");
        let queries: Vec<String> = (0..200_000)
            .map(|_| format!("{}.{}.com", rng.label(), rng.label()))
            .collect();
        let t = Instant::now();
        let mut hits = 0usize;
        for q in &queries {
            hits += usize::from(ac.is_match(black_box(q.as_bytes())));
        }
        println!(
            "acbuild keywords={n} kind={kind:?} build_ms p50={:.2} heap_bytes={} match_ns={:.1} hits={hits}",
            percentile(times, 0.5),
            ac.memory_usage(),
            per_op(t.elapsed(), queries.len())
        );
    }
}
