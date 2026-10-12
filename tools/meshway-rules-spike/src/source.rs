//! Source parsers with per-entry reports: every input entry is either
//! converted to one of exact／suffix／keyword／CIDR or counted as unconverted
//! with a reason. Nothing aborts the whole source except an unreadable document.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    SingBox,
    MihomoDomainYaml,
    MihomoIpcidrYaml,
    MihomoDomainText,
    MihomoIpcidrText,
    Classical,
}

impl Format {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "sing-box" => Self::SingBox,
            "mihomo-domain" => Self::MihomoDomainYaml,
            "mihomo-ipcidr" => Self::MihomoIpcidrYaml,
            "mihomo-domain-text" => Self::MihomoDomainText,
            "mihomo-ipcidr-text" => Self::MihomoIpcidrText,
            "classical" => Self::Classical,
            _ => return None,
        })
    }
}

/// Converted entries before minimisation, plus the report.
#[derive(Default)]
pub struct Parsed {
    pub exact: Vec<String>,
    pub suffix: Vec<String>,
    /// Proper subdomains only (sing-box `.example.com`, mihomo `.example.com`).
    pub subdomain: Vec<String>,
    pub keyword: Vec<String>,
    pub cidr: Vec<(IpAddr, u8)>,
    pub report: Report,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub input_entries: usize,
    pub exact: usize,
    pub suffix: usize,
    pub subdomain: usize,
    pub keyword: usize,
    pub regex_total: usize,
    /// Regexes that are exactly an exact／suffix／keyword pattern and were lowered.
    pub regex_lowered: usize,
    pub cidr: usize,
    /// Unconverted entries by reason (regex, subdomain-only, wildcard, process, ...).
    pub unconverted: BTreeMap<String, usize>,
    pub samples: BTreeMap<String, Vec<String>>,
}

impl Report {
    fn skip(&mut self, reason: &str, entry: &str) {
        *self.unconverted.entry(reason.to_owned()).or_default() += 1;
        let samples = self.samples.entry(reason.to_owned()).or_default();
        if samples.len() < 3 {
            samples.push(entry.to_owned());
        }
    }
}

pub fn parse(bytes: &[u8], format: Format) -> Result<Parsed, String> {
    let mut out = Parsed::default();
    match format {
        Format::SingBox => singbox(bytes, &mut out)?,
        Format::MihomoDomainYaml => {
            for entry in yaml_payload(bytes)? {
                mihomo_domain(&entry, &mut out);
            }
        }
        Format::MihomoIpcidrYaml => {
            for entry in yaml_payload(bytes)? {
                out.report.input_entries += 1;
                cidr(&entry, &mut out);
            }
        }
        Format::MihomoDomainText => {
            for entry in text_lines(bytes)? {
                mihomo_domain(entry, &mut out);
            }
        }
        Format::MihomoIpcidrText => {
            for entry in text_lines(bytes)? {
                out.report.input_entries += 1;
                cidr(entry, &mut out);
            }
        }
        Format::Classical => {
            for line in text_lines(bytes)? {
                out.report.input_entries += 1;
                let mut parts = line.split(',');
                let kind = parts.next().unwrap_or_default().trim();
                let value = parts.next().unwrap_or_default().trim();
                match kind {
                    "DOMAIN" => name(value, Kind::Exact, &mut out),
                    "DOMAIN-SUFFIX" => name(value, Kind::Suffix, &mut out),
                    "DOMAIN-KEYWORD" => keyword(value, &mut out),
                    "DOMAIN-REGEX" => regex(value, &mut out),
                    "IP-CIDR" | "IP-CIDR6" => cidr(value, &mut out),
                    other => out.report.skip(&format!("classical:{other}"), line),
                }
            }
        }
    }
    Ok(out)
}

fn yaml_payload(bytes: &[u8]) -> Result<Vec<String>, String> {
    #[derive(serde::Deserialize)]
    struct Doc {
        payload: Vec<String>,
    }
    serde_yaml_ng::from_slice::<Doc>(bytes)
        .map(|d| d.payload)
        .map_err(|e| e.to_string())
}

fn text_lines(bytes: &[u8]) -> Result<impl Iterator<Item = &str>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#')))
}

fn mihomo_domain(entry: &str, out: &mut Parsed) {
    out.report.input_entries += 1;
    if let Some(rest) = entry.strip_prefix("+.") {
        name(rest, Kind::Suffix, out)
    } else if entry.starts_with("*.") || entry.contains('*') || entry.contains('+') {
        out.report.skip("mihomo:wildcard", entry)
    } else if let Some(rest) = entry.strip_prefix('.') {
        name(rest, Kind::Subdomain, out)
    } else {
        name(entry, Kind::Exact, out)
    }
}

fn listable(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => vec![s.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}

fn singbox(bytes: &[u8], out: &mut Parsed) -> Result<(), String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let rules = doc
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("missing rules")?;
    for rule in rules {
        let Some(fields) = rule.as_object() else {
            out.report.skip("sing-box:non-object", &rule.to_string());
            continue;
        };
        let names = ["domain", "domain_suffix", "domain_keyword", "domain_regex"];
        let has_name = names.iter().any(|k| fields.contains_key(*k));
        let has_ip = fields.contains_key("ip_cidr");
        let unknown: Vec<&String> = fields
            .keys()
            .filter(|k| !names.contains(&k.as_str()) && k.as_str() != "ip_cidr")
            .collect();
        // sing-box ANDs categories inside a rule: only pure-name or pure-IP
        // rules are unions we can represent; anything else is reported whole.
        if (has_name && has_ip) || !unknown.is_empty() {
            let count: usize = fields.values().map(|v| listable(v).len().max(1)).sum();
            out.report.input_entries += count;
            let reason = if unknown.is_empty() {
                "sing-box:mixed-name-ip-rule".to_owned()
            } else {
                format!("sing-box:field:{}", unknown[0])
            };
            for _ in 0..count {
                out.report.skip(&reason, &rule.to_string());
            }
            continue;
        }
        for (key, value) in fields {
            for entry in listable(value) {
                out.report.input_entries += 1;
                match key.as_str() {
                    "domain" => name(&entry, Kind::Exact, out),
                    "domain_suffix" => {
                        if let Some(rest) = entry.strip_prefix('.') {
                            name(rest, Kind::Subdomain, out)
                        } else {
                            name(&entry, Kind::Suffix, out)
                        }
                    }
                    "domain_keyword" => keyword(&entry, out),
                    "domain_regex" => regex(&entry, out),
                    _ => cidr(&entry, out),
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Kind {
    Exact,
    Suffix,
    Subdomain,
}

fn name(raw: &str, kind: Kind, out: &mut Parsed) {
    match canonical_domain(raw) {
        Some(n) => match kind {
            Kind::Exact => {
                out.report.exact += 1;
                out.exact.push(n)
            }
            Kind::Suffix => {
                out.report.suffix += 1;
                out.suffix.push(n)
            }
            Kind::Subdomain => {
                out.report.subdomain += 1;
                out.subdomain.push(n)
            }
        },
        None => out.report.skip("invalid-name", raw),
    }
}

fn keyword(raw: &str, out: &mut Parsed) {
    let k = raw.to_ascii_lowercase();
    if k.is_empty() || !k.is_ascii() {
        out.report.skip("invalid-keyword", raw);
    } else {
        out.report.keyword += 1;
        out.keyword.push(k);
    }
}

/// Only regexes that are literally an exact, suffix or keyword pattern are
/// lowered; every other regex is reported and skipped.
fn regex(raw: &str, out: &mut Parsed) {
    out.report.regex_total += 1;
    let literal = |s: &str| -> Option<String> {
        let mut text = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next()? {
                    '.' => text.push('.'),
                    '-' => text.push('-'),
                    _ => return None,
                },
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' => text.push(c.to_ascii_lowercase()),
                _ => return None,
            }
        }
        (!text.is_empty()).then_some(text)
    };
    let lowered = if let Some(body) = raw
        .strip_prefix("(^|\\.)")
        .and_then(|r| r.strip_suffix('$'))
    {
        literal(body)
            .and_then(|l| canonical_domain(&l))
            .map(|n| (Some(Kind::Suffix), n))
    } else if let Some(body) = raw.strip_prefix('^').and_then(|r| r.strip_suffix('$')) {
        literal(body)
            .and_then(|l| canonical_domain(&l))
            .map(|n| (Some(Kind::Exact), n))
    } else if !raw.starts_with('^') && !raw.ends_with('$') {
        literal(raw).map(|l| (None, l))
    } else {
        None
    };
    match lowered {
        Some((kind, n)) => {
            out.report.regex_lowered += 1;
            match kind {
                Some(Kind::Exact) => {
                    out.report.exact += 1;
                    out.exact.push(n)
                }
                Some(Kind::Suffix | Kind::Subdomain) => {
                    out.report.suffix += 1;
                    out.suffix.push(n)
                }
                None => {
                    out.report.keyword += 1;
                    out.keyword.push(n)
                }
            }
        }
        None => out.report.skip("regex", raw),
    }
}

fn cidr(raw: &str, out: &mut Parsed) {
    let parsed = raw.split_once('/').and_then(|(a, b)| {
        let a: IpAddr = a.parse().ok()?;
        let b: u8 = b.parse().ok()?;
        let max = if a.is_ipv4() { 32 } else { 128 };
        (b <= max).then_some((a, b))
    });
    match parsed {
        Some(p) => {
            out.report.cidr += 1;
            out.cidr.push(p)
        }
        None => out.report.skip("invalid-cidr", raw),
    }
}

/// Same canonical form as `meshway_server::resources::rulesets::canonical_domain`.
pub fn canonical_domain(name: &str) -> Option<String> {
    let name = idna::domain_to_ascii(name.trim_end_matches('.'))
        .ok()?
        .to_ascii_lowercase();
    let ok = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    ok.then_some(name)
}
