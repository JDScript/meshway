//! One bounded collection and typed reading of the host facts that both the
//! readiness probe and role installation judge: sysctls, the nft ruleset and
//! the legacy iptables table list. Each check keeps its own rules and its own
//! timing; a probe result is never reused as installation permission.
use super::command::{Commands, Tool};
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// The tool or file could not be read.
    Unavailable,
    /// Output outside the expected shape or bounds.
    Malformed,
}

/// A bounded unsigned sysctl value.
pub(super) fn sysctl(path: impl AsRef<std::path::Path>) -> Result<u32, Failure> {
    let file = std::fs::File::open(path).map_err(|_| Failure::Unavailable)?;
    let mut text = String::new();
    file.take(32)
        .read_to_string(&mut text)
        .map_err(|_| Failure::Unavailable)?;
    if text.len() >= 32 {
        return Err(Failure::Malformed);
    }
    text.trim().parse().map_err(|_| Failure::Malformed)
}

/// Reverse-path filtering for one interface: the kernel applies the maximum of
/// `all` and the interface value, so strict (1) only wins when neither is loose
/// (2); values outside the kernel's range are invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReversePath {
    Usable,
    Strict,
    Invalid,
}

pub(super) fn reverse_path(all: u32, interface: u32, all_mark: u32, mark: u32) -> ReversePath {
    if all > 2 || interface > 2 || all_mark > 1 || mark > 1 {
        return ReversePath::Invalid;
    }
    if all.max(interface) == 1 {
        return ReversePath::Strict;
    }
    ReversePath::Usable
}

/// One ruleset object outside the owner's own tables.
pub(super) struct Object {
    pub kind: String,
    pub family: String,
    pub data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    nftables: Vec<Value>,
}

/// `nft -j list ruleset`, decoded once: every object except metainfo and the
/// tables in `owned`. Each item must be a single typed object with a family
/// and its table.
pub(super) async fn foreign_ruleset(
    commands: &mut Commands,
    owned: &[(String, String)],
) -> Result<Vec<Object>, Failure> {
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "ruleset".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| Failure::Unavailable)?;
    if !output.success {
        return Err(Failure::Unavailable);
    }
    decode_ruleset(&output.stdout, owned)
}

fn decode_ruleset(bytes: &[u8], owned: &[(String, String)]) -> Result<Vec<Object>, Failure> {
    let document: Document = serde_json::from_slice(bytes).map_err(|_| Failure::Malformed)?;
    if document.nftables.len() > 16384 {
        return Err(Failure::Malformed);
    }
    let mut objects = Vec::new();
    for item in document.nftables {
        if item.get("metainfo").is_some() {
            continue;
        }
        let Value::Object(object) = item else {
            return Err(Failure::Malformed);
        };
        if object.len() != 1 {
            return Err(Failure::Malformed);
        }
        let (kind, data) = object.into_iter().next().ok_or(Failure::Malformed)?;
        let family = data
            .get("family")
            .and_then(Value::as_str)
            .ok_or(Failure::Malformed)?
            .to_owned();
        let table = data
            .get(if kind == "table" { "name" } else { "table" })
            .and_then(Value::as_str)
            .ok_or(Failure::Malformed)?;
        if owned
            .iter()
            .any(|(owned_family, owned_name)| *owned_family == family && owned_name == table)
        {
            continue;
        }
        objects.push(Object { kind, family, data });
    }
    Ok(objects)
}

/// Whether any key named in `names` occurs anywhere inside `value`.
pub(super) fn contains(value: &Value, names: &[&str]) -> bool {
    match value {
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| names.contains(&key.as_str()) || contains(value, names)),
        Value::Array(values) => values.iter().any(|value| contains(value, names)),
        _ => false,
    }
}

/// Whether any `ct`/`meta` expression reads or writes the mark.
pub(super) fn uses_mark(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields
                .get("ct")
                .or_else(|| fields.get("meta"))
                .is_some_and(|expr| expr.get("key").and_then(Value::as_str) == Some("mark"))
                || fields.values().any(uses_mark)
        }
        Value::Array(values) => values.iter().any(uses_mark),
        _ => false,
    }
}

/// Loaded legacy iptables tables; an absent list means none are loaded.
pub(super) fn legacy_tables() -> Result<Vec<String>, Failure> {
    match std::fs::read_to_string("/proc/net/ip_tables_names") {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(Failure::Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ruleset_decoding_excludes_owned_tables_and_rejects_untyped_items() {
        let owned = vec![("inet".to_owned(), "meshway_own".to_owned())];
        let document = json!({"nftables": [
            {"metainfo": {"version": "1.0.9"}},
            {"table": {"family": "inet", "name": "meshway_own", "handle": 1}},
            {"rule": {"family": "inet", "table": "meshway_own", "chain": "c", "expr": [{"mangle": {}}]}},
            {"rule": {"family": "ip", "table": "filter", "chain": "c", "expr": [{"meta": {"key": "mark"}}]}},
        ]});
        let objects = decode_ruleset(&serde_json::to_vec(&document).unwrap(), &owned).unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(
            (objects[0].kind.as_str(), objects[0].family.as_str()),
            ("rule", "ip")
        );
        assert!(uses_mark(&objects[0].data));
        assert!(!contains(&objects[0].data, &["mangle"]));
        let untyped = json!({"nftables": [{"rule": {}, "chain": {}}]});
        assert!(decode_ruleset(&serde_json::to_vec(&untyped).unwrap(), &owned).is_err());
        assert!(decode_ruleset(br#"{"nftables": [{"rule": {"family": "ip"}}]}"#, &owned).is_err());
    }

    #[test]
    fn reverse_path_classification() {
        assert_eq!(reverse_path(0, 2, 0, 1), ReversePath::Usable);
        assert_eq!(reverse_path(1, 0, 0, 0), ReversePath::Strict);
        assert_eq!(reverse_path(0, 1, 0, 0), ReversePath::Strict);
        // The kernel takes max(all, interface): loose on either side overrides strict.
        assert_eq!(reverse_path(2, 1, 0, 0), ReversePath::Usable);
        assert_eq!(reverse_path(1, 2, 0, 0), ReversePath::Usable);
        assert_eq!(reverse_path(3, 0, 0, 0), ReversePath::Invalid);
        assert_eq!(reverse_path(0, 0, 2, 0), ReversePath::Invalid);
    }
}
