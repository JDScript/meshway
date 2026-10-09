use super::command::{Commands, Tool};
use crate::{Error, Result};
use serde::Deserialize;
use serde_json::Value;
use std::io::Read;

/// This backend cannot preserve opaque foreign mark writes or flow offload.
/// Refusing installation preserves those rules rather than rewriting them.
pub(super) async fn check(commands: &mut Commands, owned: &[(String, String)]) -> Result<()> {
    if sysctl("/proc/sys/net/ipv4/ip_forward")? != 1 {
        return Err(Error::Target);
    }
    let all = sysctl("/proc/sys/net/ipv4/conf/all/rp_filter")?;
    let all_mark = sysctl("/proc/sys/net/ipv4/conf/all/src_valid_mark")?;
    if all > 2 || all_mark > 1 {
        return Err(Error::Target);
    }
    for (index, entry) in std::fs::read_dir("/proc/sys/net/ipv4/conf")?.enumerate() {
        if index >= 256 {
            return Err(Error::Capacity);
        }
        let entry = entry?;
        let name = entry.file_name();
        if name == "all" {
            continue;
        }
        let rpf = sysctl(entry.path().join("rp_filter"))?;
        let mark = sysctl(entry.path().join("src_valid_mark"))?;
        if rpf > 2 || mark > 1 || all.max(rpf) == 1 {
            return Err(Error::Conflict);
        }
    }
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "ruleset".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| Error::Netlink)?;
    if !output.success {
        return Err(Error::Netlink);
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Ruleset {
        nftables: Vec<Value>,
    }
    let rules: Ruleset = serde_json::from_slice(&output.stdout).map_err(|_| Error::Conflict)?;
    if rules.nftables.len() > 16384 {
        return Err(Error::Capacity);
    }
    if rules
        .nftables
        .iter()
        .filter(|item| {
            !item.as_object().is_some_and(|object| {
                object.values().any(|data| {
                    let family = data.get("family").and_then(Value::as_str);
                    let table = data
                        .get("table")
                        .or_else(|| data.get("name"))
                        .and_then(Value::as_str);
                    owned.iter().any(|(own_family, own_table)| {
                        family == Some(own_family.as_str()) && table == Some(own_table.as_str())
                    })
                })
            })
        })
        .any(conflicts)
    {
        return Err(Error::Conflict);
    }
    if let Ok(text) = std::fs::read_to_string("/proc/net/ip_tables_names")
        && text
            .lines()
            .any(|line| matches!(line.trim(), "mangle" | "raw" | "nat"))
    {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn sysctl(path: impl AsRef<std::path::Path>) -> Result<u32> {
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(32)
        .read_to_string(&mut text)?;
    if text.len() >= 32 {
        return Err(Error::Target);
    }
    text.trim().parse().map_err(|_| Error::Target)
}

fn conflicts(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(conflicts),
        Value::Object(values) => {
            values.contains_key("flowtable")
                || values.contains_key("flow")
                || values.contains_key("mangle")
                || values.contains_key("ct helper")
                || values
                    .get("ct")
                    .or_else(|| values.get("meta"))
                    .is_some_and(|expr| expr.get("key").and_then(Value::as_str) == Some("mark"))
                || values.values().any(conflicts)
        }
        _ => false,
    }
}
