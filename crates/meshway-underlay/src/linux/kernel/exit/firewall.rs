use super::{
    ExitError,
    command::Commands,
    host::{self, Failure},
};
use meshway_model::exit::ExitUnavailableReason as Reason;

/// Readiness observation. Unknown blocking hooks, mark manipulation and
/// offload are not an available IPv4 exit prerequisite. Never rewrite a host
/// firewall to obtain readiness.
pub(super) async fn check(
    commands: &mut Commands,
    owned: &[(String, String)],
) -> Result<(), ExitError> {
    let foreign = host::foreign_ruleset(commands, owned)
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    for object in foreign {
        if !matches!(
            object.family.as_str(),
            "inet" | "ip" | "bridge" | "netdev" | "arp"
        ) {
            continue;
        }
        if host::contains(&object.data, &["mangle", "flow", "ct helper"])
            || object.kind == "flowtable"
        {
            return Err(Reason::OwnershipConflict.into());
        }
        if host::contains(
            &object.data,
            &["drop", "reject", "queue", "tproxy", "dup", "fwd"],
        ) || object
            .data
            .get("policy")
            .and_then(serde_json::Value::as_str)
            == Some("drop")
        {
            return Err(Reason::FirewallConflict.into());
        }
    }
    match host::legacy_tables() {
        Ok(tables) if tables.is_empty() => Ok(()),
        Ok(_) | Err(Failure::Unavailable | Failure::Malformed) => {
            Err(Reason::FirewallConflict.into())
        }
    }
}
