use super::{
    ExitError,
    command::{Commands, Tool},
};
use meshway_model::exit::ExitUnavailableReason as Reason;
use serde::Deserialize;
use serde_json::Value;

/// Unknown blocking hooks, mark manipulation and offload are not an available
/// IPv4 exit prerequisite. Never rewrite a host firewall to obtain readiness.
pub(super) async fn check(
    commands: &mut Commands,
    owned: &[(String, String)],
) -> Result<(), ExitError> {
    let output = commands
        .run(
            Tool::Nft,
            vec!["-j".into(), "list".into(), "ruleset".into()],
            Vec::new(),
        )
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    if !output.success {
        return Err(Reason::NftUnavailable.into());
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Ruleset {
        nftables: Vec<Value>,
    }
    let rules: Ruleset =
        serde_json::from_slice(&output.stdout).map_err(|_| Reason::NftUnavailable)?;
    if rules.nftables.len() > 16384 {
        return Err(Reason::NftUnavailable.into());
    }
    for item in rules.nftables {
        if item.get("metainfo").is_some() {
            continue;
        }
        let object = item
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or(Reason::NftUnavailable)?;
        let (kind, data) = object.iter().next().ok_or(Reason::NftUnavailable)?;
        let family = data
            .get("family")
            .and_then(Value::as_str)
            .ok_or(Reason::NftUnavailable)?;
        let table = data
            .get(if kind == "table" { "name" } else { "table" })
            .and_then(Value::as_str)
            .ok_or(Reason::NftUnavailable)?;
        if owned
            .iter()
            .any(|(owned_family, owned_name)| owned_family == family && owned_name == table)
        {
            continue;
        }
        if !matches!(family, "inet" | "ip" | "bridge" | "netdev" | "arp") {
            continue;
        }
        if contains(data, &["mangle", "flow", "ct helper"]) || kind == "flowtable" {
            return Err(Reason::OwnershipConflict.into());
        }
        if contains(data, &["drop", "reject", "queue", "tproxy", "dup", "fwd"])
            || data.get("policy").and_then(Value::as_str) == Some("drop")
        {
            return Err(Reason::FirewallConflict.into());
        }
    }
    match std::fs::read_to_string("/proc/net/ip_tables_names") {
        Ok(names) if !names.trim().is_empty() => return Err(Reason::FirewallConflict.into()),
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(Reason::FirewallConflict.into());
        }
        _ => {}
    }
    Ok(())
}
fn contains(value: &Value, names: &[&str]) -> bool {
    match value {
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| names.contains(&key.as_str()) || contains(value, names)),
        Value::Array(values) => values.iter().any(|value| contains(value, names)),
        _ => false,
    }
}
