//! Whether a foreign firewall can stop this exit's traffic. Only a rule that
//! can match that traffic is a conflict: a blocking verdict or `policy drop`
//! on the prerouting (raw, mangle, nat and filter priorities alike),
//! forward, output or postrouting hook whose interface and destination
//! constraints do not exclude the exit TUN, the default-route interfaces and
//! the addresses exit packets carry. Rules confined to other interfaces or
//! addresses (Docker's bridges and containers, say) are not. Never rewrite a
//! host firewall to obtain readiness.
use super::{
    ExitError,
    command::{Commands, Tool},
    host::{self, Failure},
};
use meshway_model::exit::ExitUnavailableReason as Reason;

mod crossing;
mod interface;
mod iptables;
mod judge;
mod nft;

pub(in crate::linux::kernel) use crossing::Crossing;
use iptables::Legacy;
use judge::Ruleset;

/// Readiness observation: a blocking foreign rule for exit traffic, foreign
/// mark manipulation and offload, read from nft and from legacy iptables.
pub(super) async fn check(
    commands: &mut Commands,
    owned: &[(String, String)],
    crossing: &Crossing,
) -> Result<(), ExitError> {
    let foreign = host::foreign_ruleset(commands, owned)
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    for object in &foreign {
        if matches!(
            object.family.as_str(),
            "inet" | "ip" | "bridge" | "netdev" | "arp"
        ) && (host::contains(&object.data, &["mangle", "flow", "ct helper"])
            || object.kind == "flowtable")
        {
            return Err(Reason::OwnershipConflict.into());
        }
    }
    let mut ruleset = Ruleset::from_nft(&foreign);
    let legacy = legacy(commands)
        .await
        .map_err(|_| Reason::FirewallConflict)?;
    if let Some(legacy) = legacy {
        if legacy.marks {
            return Err(Reason::OwnershipConflict.into());
        }
        ruleset.chains.extend(legacy.ruleset.chains);
    }
    ruleset.judge(crossing).map_err(Into::into)
}

/// The loaded legacy iptables tables, parsed; `None` when none are loaded.
/// Loaded tables that cannot be read cannot be judged.
pub(super) async fn legacy(commands: &mut Commands) -> Result<Option<Legacy>, Failure> {
    if host::legacy_tables()?.is_empty() {
        return Ok(None);
    }
    let output = commands
        .run(Tool::IptablesLegacySave, Vec::new(), Vec::new())
        .await
        .map_err(|_| Failure::Unavailable)?;
    if !output.success {
        return Err(Failure::Unavailable);
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| Failure::Malformed)?;
    Legacy::parse(text).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(super) fn crossing() -> Crossing {
        Crossing {
            egress: vec!["eth0".into()],
            present: ["lo", "eth0", "docker0", "br-35ac0d23089d", "tun0"]
                .map(String::from)
                .to_vec(),
            installed: None,
            local: vec!["104.194.68.10".parse().unwrap()],
            mesh: vec!["100.64.0.0/10".parse().unwrap()],
        }
    }

    pub(super) fn nft(items: Value) -> Ruleset {
        let document = json!({"nftables": items});
        let objects = host::decode_ruleset(&serde_json::to_vec(&document).unwrap(), &[]).unwrap();
        Ruleset::from_nft(&objects)
    }

    pub(super) fn chain(
        family: &str,
        table: &str,
        name: &str,
        hook: Option<&str>,
        policy: &str,
    ) -> Value {
        let mut chain = json!({"family": family, "table": table, "name": name});
        if let Some(hook) = hook {
            chain["type"] = json!("filter");
            chain["hook"] = json!(hook);
            chain["prio"] = json!(0);
            chain["policy"] = json!(policy);
        }
        json!({"chain": chain})
    }

    pub(super) fn rule(family: &str, table: &str, chain: &str, expr: Value) -> Value {
        json!({"rule": {"family": family, "table": table, "chain": chain, "expr": expr}})
    }

    pub(super) fn name(key: &str, op: &str, right: Value) -> Value {
        json!({"match": {"op": op, "left": {"meta": {"key": key}}, "right": right}})
    }
}
