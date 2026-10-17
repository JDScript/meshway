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
use meshway_model::exit::{ExitFamily, ExitUnavailableReason as Reason};

mod crossing;
mod interface;
mod iptables;
mod judge;
mod nft;

pub(in crate::linux::kernel) use crossing::Crossing;
use iptables::Legacy;
use judge::Ruleset;

/// Readiness observation: a blocking foreign rule for exit traffic of each
/// of `families`, foreign mark manipulation and offload, read from nft and
/// from legacy iptables/ip6tables. `Err` fails every family (unreadable
/// ruleset, foreign marks or offload); otherwise each family is judged on
/// its own packets.
pub(super) async fn check(
    commands: &mut Commands,
    owned: &[(String, String)],
    crossing: &Crossing,
    families: &[ExitFamily],
) -> Result<Vec<(ExitFamily, Result<(), Reason>)>, ExitError> {
    let foreign = host::foreign_ruleset(commands, owned)
        .await
        .map_err(|_| Reason::NftUnavailable)?;
    for object in &foreign {
        if matches!(
            object.family.as_str(),
            "inet" | "ip" | "ip6" | "bridge" | "netdev" | "arp"
        ) && (host::contains(&object.data, &["mangle", "flow", "ct helper"])
            || object.kind == "flowtable")
        {
            return Err(Reason::OwnershipConflict.into());
        }
    }
    let mut ruleset = Ruleset::from_nft(&foreign);
    let mut unread = Vec::new();
    for &family in families {
        match legacy(commands, family).await {
            Ok(Some(legacy)) if legacy.marks => return Err(Reason::OwnershipConflict.into()),
            Ok(Some(legacy)) => ruleset.chains.extend(legacy.ruleset.chains),
            Ok(None) => {}
            // Only this family's legacy tables are unreadable.
            Err(_) => unread.push(family),
        }
    }
    Ok(families
        .iter()
        .map(|family| {
            let judged = if unread.contains(family) {
                Err(Reason::FirewallConflict)
            } else {
                ruleset.judge(crossing, *family)
            };
            (*family, judged)
        })
        .collect())
}

/// The loaded legacy iptables (IPv4) or ip6tables (IPv6) tables, parsed;
/// `None` when none are loaded. Loaded tables that cannot be read cannot be
/// judged.
pub(super) async fn legacy(
    commands: &mut Commands,
    family: ExitFamily,
) -> Result<Option<Legacy>, Failure> {
    let ipv6 = family == ExitFamily::Ipv6;
    if host::legacy_tables(ipv6)?.is_empty() {
        return Ok(None);
    }
    let (tool, source) = if ipv6 {
        (Tool::Ip6tablesLegacySave, "legacy6")
    } else {
        (Tool::IptablesLegacySave, "legacy")
    };
    let output = commands
        .run(tool, Vec::new(), Vec::new())
        .await
        .map_err(|_| Failure::Unavailable)?;
    if !output.success {
        return Err(Failure::Unavailable);
    }
    let text = std::str::from_utf8(&output.stdout).map_err(|_| Failure::Malformed)?;
    Legacy::parse(text, source).map(Some)
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
            egress6: vec!["eth0".into()],
            local6: vec!["3ffe:1:2:3::10".parse().unwrap()],
            advertised6: Vec::new(),
            mesh: vec![
                "100.64.0.0/10".parse().unwrap(),
                "fd79:5c1e:e325::/48".parse().unwrap(),
            ],
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
