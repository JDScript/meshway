//! R1 differential: the production `meshway-rules` converter and matcher
//! against this spike's parser, canonicalizer and reference matcher on every
//! R0 input. Prints one JSON line per set; any mismatch is listed.

use crate::format::{self, Canonical, Reference};
use crate::source::{self, Format};
use meshway_rules::convert::{self, Format as R1Format};
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

#[derive(Serialize)]
struct Row {
    set: String,
    r1_digest: String,
    r1_bytes: usize,
    spike_digest: String,
    /// exact, suffix, subdomain, keyword, v4, v6
    r1_counts: [u32; 6],
    spike_counts: [usize; 6],
    r1_skipped: u64,
    spike_unconverted: usize,
    domain_queries: usize,
    domain_mismatches: usize,
    ip_queries: usize,
    ip_mismatches: usize,
    samples: Vec<String>,
}

fn r1_format(f: Format) -> R1Format {
    match f {
        Format::SingBox => R1Format::SingBoxSource,
        Format::MihomoDomainYaml => R1Format::MihomoDomain,
        Format::MihomoIpcidrYaml => R1Format::MihomoIpcidr,
        Format::MihomoDomainText => R1Format::MihomoDomainText,
        Format::MihomoIpcidrText => R1Format::MihomoIpcidrText,
        Format::Classical => R1Format::Classical,
    }
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
        let len = 1 + self.below(10);
        (0..len)
            .map(|_| b"abcdefghijklmnopqrstuvwxyz0123456789-"[self.below(37)] as char)
            .collect()
    }
}

fn domain_queries(c: &Canonical, rng: &mut Rng, random: usize) -> Vec<String> {
    let mut q = vec![];
    let names: Vec<&String> = c.exact.iter().chain(&c.suffix).chain(&c.subdomain).collect();
    for n in &names {
        q.push((*n).clone());
        q.push(format!("{}.{n}", rng.label()));
        q.push(format!("{}{n}", rng.label()));
        if let Some((_, parent)) = n.split_once('.') {
            q.push(parent.to_owned());
        }
    }
    for k in &c.keyword {
        q.push(format!("{}{k}{}.com", rng.label(), rng.label()));
        q.push(k.clone());
    }
    let tlds = ["com", "net", "org", "cn", "io", "xyz", "co.uk", "com.cn"];
    for _ in 0..random {
        let labels = 1 + rng.below(3);
        let mut name = (0..labels).map(|_| rng.label()).collect::<Vec<_>>().join(".");
        name.push('.');
        name.push_str(tlds[rng.below(tlds.len())]);
        q.push(name);
        if !names.is_empty() {
            q.push(format!("{}.{}", rng.label(), names[rng.below(names.len())]));
        }
    }
    q
}

fn ip_queries(c: &Canonical, rng: &mut Rng, random: usize) -> Vec<IpAddr> {
    let mut q = vec![];
    for (s, e) in &c.v4 {
        for v in [s.wrapping_sub(1), *s, *e, e.wrapping_add(1)] {
            q.push(IpAddr::V4(Ipv4Addr::from(v)));
        }
    }
    for (s, e) in &c.v6 {
        for v in [s.wrapping_sub(1), *s, *e, e.wrapping_add(1)] {
            q.push(IpAddr::V6(Ipv6Addr::from(v)));
        }
    }
    for _ in 0..random {
        q.push(IpAddr::V4(Ipv4Addr::from(rng.next() as u32)));
        q.push(IpAddr::V6(Ipv6Addr::from(
            (0x2000u128 << 112) | (u128::from(rng.next()) << 48) | u128::from(rng.next() >> 16),
        )));
    }
    q
}

pub fn run(root: &Path, sources: &[(&str, &str, &str, Option<&str>)], random: usize) {
    let out = root.join("out-r1");
    std::fs::create_dir_all(&out).expect("out-r1");
    let mut total = 0;
    for (name, file, format, _) in sources {
        let format = Format::parse(format).expect("format");
        let bytes = std::fs::read(root.join(file)).expect("source");
        let parsed = source::parse(&bytes, format).expect("spike parse");
        let c = format::canonicalize(&parsed);
        let spike = format::build(&c);
        let reference = Reference { c: &c };
        let conversion = convert::convert(&bytes, r1_format(format)).expect("r1 convert");
        std::fs::write(out.join(format!("{name}.mwrs")), conversion.artifact.bytes()).expect("write");
        let r1 = conversion.artifact.clone().open().expect("r1 open");
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let mut samples = vec![];
        let dq = domain_queries(&c, &mut rng, random);
        let mut dm = 0;
        for q in &dq {
            let (want, got) = (reference.domain(q), r1.match_domain(q).is_some());
            if want != got {
                dm += 1;
                if samples.len() < 10 {
                    samples.push(format!("domain {q}: spike={want} r1={got}"));
                }
            }
        }
        let iq = ip_queries(&c, &mut rng, random);
        let mut im = 0;
        for a in &iq {
            let (want, got) = (reference.ip(*a), r1.match_ip(*a));
            if want != got {
                im += 1;
                if samples.len() < 10 {
                    samples.push(format!("ip {a}: spike={want} r1={got}"));
                }
            }
        }
        total += dm + im;
        let k = r1.counts();
        let row = Row {
            set: name.to_string(),
            r1_digest: r1.digest().to_string(),
            r1_bytes: r1.as_bytes().len(),
            spike_digest: format!("sha256:{}", format::digest(&spike)),
            r1_counts: [k.exact, k.suffix, k.subdomain, k.keyword, k.ipv4, k.ipv6],
            spike_counts: [
                c.exact.len(),
                c.suffix.len(),
                c.subdomain.len(),
                c.keyword.len(),
                c.v4.len(),
                c.v6.len(),
            ],
            r1_skipped: conversion.report.summary.skipped(),
            spike_unconverted: parsed.report.unconverted.values().sum(),
            domain_queries: dq.len(),
            domain_mismatches: dm,
            ip_queries: iq.len(),
            ip_mismatches: im,
            samples,
        };
        println!("{}", serde_json::to_string(&row).expect("json"));
    }
    println!("{{\"total_mismatches\":{total}}}");
}
