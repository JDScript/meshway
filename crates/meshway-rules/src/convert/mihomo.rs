use super::{cidr, keyword, name};
use crate::{ConvertError, Entry, Reason, report::Collector};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    payload: Vec<serde_yaml_ng::Value>,
}

pub(super) fn payload(bytes: &[u8]) -> Result<Vec<serde_yaml_ng::Value>, ConvertError> {
    serde_yaml_ng::from_slice::<Payload>(bytes)
        .map(|p| p.payload)
        .map_err(|e| ConvertError::InvalidSource(e.to_string()))
}

pub(super) fn show(item: &serde_yaml_ng::Value) -> String {
    serde_yaml_ng::to_string(item)
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// Text format: one item per line, surrounding whitespace ignored, empty and
/// `#` lines skipped. Yields 1-based line numbers.
pub(super) fn lines(bytes: &[u8]) -> Result<impl Iterator<Item = (usize, &str)>, ConvertError> {
    let text =
        std::str::from_utf8(bytes).map_err(|e| ConvertError::InvalidSource(e.to_string()))?;
    Ok(text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty() && !l.starts_with('#')))
}

/// `+.example.com` suffix, `.example.com` subdomain, bare name exact; `*`
/// (one wildcard label) and any other wildcard have no equivalent.
pub(super) fn domain(text: &str, at: impl FnOnce() -> String, c: &mut Collector) {
    if text.contains('*') {
        return c.report.skip(at, Reason::Wildcard, text);
    }
    if let Some(rest) = text.strip_prefix("+.") {
        if rest.contains('+') {
            return c.report.skip(at, Reason::Wildcard, text);
        }
        return name(rest, Entry::Suffix, at, c);
    }
    if text.contains('+') {
        return c.report.skip(at, Reason::Wildcard, text);
    }
    match text.strip_prefix('.') {
        Some(rest) => name(rest, Entry::Subdomain, at, c),
        None => name(text, Entry::Exact, at, c),
    }
}

/// `TYPE,value[,option...]`. Only domain, keyword, regex (skipped) and
/// `IP-CIDR`／`IP-CIDR6` (option `no-resolve` only) have an equivalent.
pub(super) fn classical(text: &str, at: impl FnOnce() -> String, c: &mut Collector) {
    let mut parts = text.split(',').map(str::trim);
    let kind = parts.next().unwrap_or_default();
    let value = parts.next().unwrap_or_default();
    let options: Vec<&str> = parts.collect();
    let unsupported = || Reason::Classical(kind.to_owned());
    match kind {
        "DOMAIN" | "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" if !options.is_empty() => {
            c.report.skip(at, unsupported(), text)
        }
        "DOMAIN" => name(value, Entry::Exact, at, c),
        "DOMAIN-SUFFIX" => name(value, Entry::Suffix, at, c),
        "DOMAIN-KEYWORD" => keyword(value, at, c),
        "DOMAIN-REGEX" => c.report.skip(at, Reason::Regex, text),
        "IP-CIDR" | "IP-CIDR6" if options.iter().all(|o| o.eq_ignore_ascii_case("no-resolve")) => {
            cidr(value, at, c)
        }
        _ => c.report.skip(at, unsupported(), text),
    }
}
