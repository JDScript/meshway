use super::{cidr, keyword, name};
use crate::{ConvertError, Entry, Reason, report::Collector};
use serde_json::Value;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    version: u8,
    rules: Vec<Value>,
}

const NAME_FIELDS: [&str; 4] = ["domain", "domain_suffix", "domain_keyword", "domain_regex"];
const IP_FIELD: &str = "ip_cidr";

/// sing-box `listable`: a string or an array of strings.
fn items(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Headless rules: fields inside one rule are ANDed by category, rules are
/// ORed. Only a rule that is purely names or purely `ip_cidr` is a union.
pub(super) fn convert(bytes: &[u8], c: &mut Collector) -> Result<(), ConvertError> {
    let source: Source =
        serde_json::from_slice(bytes).map_err(|e| ConvertError::InvalidSource(e.to_string()))?;
    if !(1..=3).contains(&source.version) {
        return Err(ConvertError::InvalidSource(format!(
            "unsupported sing-box rule-set version {}",
            source.version
        )));
    }
    for (r, rule) in source.rules.iter().enumerate() {
        let Value::Object(fields) = rule else {
            return Err(ConvertError::InvalidSource(format!(
                "rule {r} is not an object"
            )));
        };
        // `type: default` is the implicit headless rule type.
        // Sorted, so reports do not depend on the JSON map implementation.
        let mut fields: Vec<(&String, &Value)> = fields
            .iter()
            .filter(|(k, v)| !(k.as_str() == "type" && v.as_str() == Some("default")))
            .collect();
        fields.sort_by_key(|(k, _)| *k);
        // No condition at all matches everything upstream.
        if fields.iter().all(|(_, v)| items(v).is_empty()) {
            c.report.skip(
                || format!("/rules/{r}"),
                Reason::EmptyRule,
                &rule.to_string(),
            );
            continue;
        }
        let unknown = fields
            .iter()
            .find(|(k, _)| !NAME_FIELDS.contains(&k.as_str()) && k.as_str() != IP_FIELD);
        let has_name = fields
            .iter()
            .any(|(k, _)| NAME_FIELDS.contains(&k.as_str()));
        let has_ip = fields.iter().any(|(k, _)| k.as_str() == IP_FIELD);
        let whole_rule = match unknown {
            Some((k, _)) => Some(Reason::UnsupportedField((*k).clone())),
            None if has_name && has_ip => Some(Reason::MixedRule),
            None => None,
        };
        for (field, value) in &fields {
            for (i, item) in items(value).into_iter().enumerate() {
                let at = || format!("/rules/{r}/{field}/{i}");
                if let Some(reason) = &whole_rule {
                    c.report.skip(at, reason.clone(), &text(item));
                    continue;
                }
                let Some(s) = item.as_str() else {
                    c.report.skip(at, Reason::NotString, &text(item));
                    continue;
                };
                match field.as_str() {
                    "domain" => name(s, Entry::Exact, at, c),
                    "domain_suffix" => match s.strip_prefix('.') {
                        Some(rest) => name(rest, Entry::Subdomain, at, c),
                        None => name(s, Entry::Suffix, at, c),
                    },
                    "domain_keyword" => keyword(s, at, c),
                    "domain_regex" => c.report.skip(at, Reason::Regex, s),
                    _ => cidr(s, at, c),
                }
            }
        }
    }
    Ok(())
}
