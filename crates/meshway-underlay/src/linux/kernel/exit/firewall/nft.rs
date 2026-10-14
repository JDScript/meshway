use super::{
    host,
    iptables::legacy_blocks,
    judge::{Hook, Match, Rule, Ruleset, Verdict},
};
use serde_json::Value;

impl Ruleset {
    pub(super) fn from_nft(objects: &[host::Object]) -> Self {
        let mut ruleset = Self::default();
        for object in objects {
            // IPv4 exit traffic: bridge, ARP and netdev hooks see no routed packet.
            if !matches!(object.family.as_str(), "ip" | "inet") {
                continue;
            }
            let field = |name: &str| object.data.get(name).and_then(Value::as_str);
            let (Some(table), Some(chain)) = (
                field("table"),
                field(if object.kind == "chain" {
                    "name"
                } else {
                    "chain"
                }),
            ) else {
                continue;
            };
            let key = (object.family.clone(), table.to_owned(), chain.to_owned());
            match object.kind.as_str() {
                "chain" => {
                    let entry = ruleset.chains.entry(key).or_default();
                    entry.hook = match field("hook") {
                        Some("prerouting") => Some(Hook::Prerouting),
                        Some("forward") => Some(Hook::Forward),
                        Some("output") => Some(Hook::Output),
                        Some("postrouting") => Some(Hook::Postrouting),
                        _ => None,
                    };
                    entry.drops = field("policy") == Some("drop");
                }
                "rule" => {
                    let expressions = object
                        .data
                        .get("expr")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    ruleset
                        .chains
                        .entry(key)
                        .or_default()
                        .rules
                        .push(nft_rule(expressions));
                }
                _ => {}
            }
        }
        ruleset
    }
}

fn strings(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::String(value) => Some(vec![value.clone()]),
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
        Value::Object(object) => object.get("set").and_then(strings),
        _ => None,
    }
}

fn nft_match(object: &Value) -> Match {
    let (Some(op), Some(left), Some(right)) = (
        object.get("op").and_then(Value::as_str),
        object.get("left"),
        object.get("right"),
    ) else {
        return Match::Unknown;
    };
    let equal = match op {
        "==" | "in" => true,
        "!=" => false,
        _ => return Match::Unknown,
    };
    let key = |kind: &str| {
        left.get(kind)
            .and_then(|value| value.get("key"))
            .and_then(Value::as_str)
    };
    if left.pointer("/payload/protocol").and_then(Value::as_str) == Some("ip")
        && left.pointer("/payload/field").and_then(Value::as_str) == Some("daddr")
    {
        return ipv4_nets(right).map_or(Match::Unknown, |nets| Match::Destination(equal, nets));
    }
    let Some(values) = strings(right) else {
        return Match::Unknown;
    };
    match (key("meta"), key("ct")) {
        (Some("iifname" | "iif"), _) => Match::Input(equal, values),
        (Some("oifname" | "oif"), _) => Match::Output(equal, values),
        (Some("nfproto"), _) => Match::Family(equal, values),
        (_, Some("state")) if left.pointer("/ct/dir").is_none() => Match::State(equal, values),
        _ => Match::Unknown,
    }
}

/// An nft address, prefix or anonymous set of them as IPv4 networks.
fn ipv4_nets(value: &Value) -> Option<Vec<ipnet::Ipv4Net>> {
    match value {
        Value::String(text) => text
            .parse::<ipnet::Ipv4Net>()
            .ok()
            .or_else(|| text.parse::<std::net::Ipv4Addr>().ok().map(Into::into))
            .map(|net| vec![net]),
        Value::Array(values) => values
            .iter()
            .map(ipv4_nets)
            .collect::<Option<Vec<_>>>()
            .map(|nets| nets.concat()),
        Value::Object(object) => {
            if let Some(set) = object.get("set") {
                return ipv4_nets(set);
            }
            let prefix = object.get("prefix")?;
            let address = prefix.get("addr")?.as_str()?.parse().ok()?;
            let length = u8::try_from(prefix.get("len")?.as_u64()?).ok()?;
            ipnet::Ipv4Net::new(address, length)
                .ok()
                .map(|net| vec![net])
        }
        _ => None,
    }
}

const BLOCKING: [&str; 6] = ["drop", "reject", "queue", "tproxy", "dup", "fwd"];

fn nft_verdict(key: &str, value: &Value) -> Option<Verdict> {
    let target = || {
        value
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Some(match key {
        key if BLOCKING.contains(&key) => Verdict::Block,
        "accept" => Verdict::Accept,
        "return" => Verdict::Return,
        "jump" => Verdict::Jump(target()?),
        "goto" => Verdict::Goto(target()?),
        "vmap" => Verdict::Map(
            value
                .pointer("/data/set")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| entry.get(1).and_then(Value::as_object))
                        .flat_map(|verdict| {
                            verdict
                                .iter()
                                .filter_map(|(key, value)| nft_verdict(key, value))
                        })
                        .collect()
                })
                .unwrap_or_else(|| vec![Verdict::Block]),
        ),
        "xt" if value.get("type").and_then(Value::as_str) == Some("target") => {
            match value.get("name").and_then(Value::as_str) {
                Some(name) if legacy_blocks(name) => Verdict::Block,
                _ => return None,
            }
        }
        _ => return None,
    })
}

fn nft_rule(expressions: &[Value]) -> Rule {
    let mut matches = Vec::new();
    for expression in expressions {
        let Some((key, value)) = expression
            .as_object()
            .and_then(|object| (object.len() == 1).then(|| object.iter().next()).flatten())
        else {
            matches.push(Match::Unknown);
            continue;
        };
        if key == "match" {
            matches.push(nft_match(value));
        } else if let Some(verdict) = nft_verdict(key, value) {
            return Rule { matches, verdict };
        } else if matches!(key.as_str(), "limit" | "quota" | "xt" | "set" | "map") {
            // These statements pass only some packets on.
            matches.push(Match::Unknown);
        }
    }
    Rule {
        matches,
        verdict: Verdict::Continue,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        crossing::Crossing,
        tests::{chain, crossing, name, nft, rule},
    };
    use super::*;
    use meshway_model::exit::ExitUnavailableReason as Reason;
    use serde_json::json;

    /// Docker 28 on iptables-nft: confined drops, accept policy (it7).
    fn docker(policy: &str) -> Ruleset {
        let r = |chain, expr| rule("ip", "filter", chain, expr);
        let bridge = "br-35ac0d23089d";
        nft(json!([
            {"table": {"family": "ip", "name": "filter"}},
            chain("ip", "filter", "FORWARD", Some("forward"), policy),
            chain("ip", "filter", "DOCKER-USER", None, ""),
            chain("ip", "filter", "DOCKER-FORWARD", None, ""),
            chain("ip", "filter", "DOCKER-CT", None, ""),
            chain("ip", "filter", "DOCKER-ISOLATION-STAGE-1", None, ""),
            chain("ip", "filter", "DOCKER-ISOLATION-STAGE-2", None, ""),
            chain("ip", "filter", "DOCKER-BRIDGE", None, ""),
            chain("ip", "filter", "DOCKER", None, ""),
            r("FORWARD", json!([{"counter": null}, {"jump": {"target": "DOCKER-USER"}}])),
            r("FORWARD", json!([{"jump": {"target": "DOCKER-FORWARD"}}])),
            r("DOCKER-FORWARD", json!([{"jump": {"target": "DOCKER-CT"}}])),
            r("DOCKER-FORWARD", json!([{"jump": {"target": "DOCKER-ISOLATION-STAGE-1"}}])),
            r("DOCKER-FORWARD", json!([{"jump": {"target": "DOCKER-BRIDGE"}}])),
            r("DOCKER-FORWARD", json!([name("iifname", "==", json!(bridge)), {"accept": null}])),
            r("DOCKER-CT", json!([name("oifname", "==", json!(bridge)), {"xt": {"type": "match", "name": "conntrack"}}, {"accept": null}])),
            r("DOCKER-ISOLATION-STAGE-1", json!([name("iifname", "==", json!(bridge)), name("oifname", "!=", json!(bridge)), {"jump": {"target": "DOCKER-ISOLATION-STAGE-2"}}])),
            r("DOCKER-ISOLATION-STAGE-2", json!([name("oifname", "==", json!("docker0")), {"drop": null}])),
            r("DOCKER-ISOLATION-STAGE-2", json!([name("oifname", "==", json!(bridge)), {"drop": null}])),
            r("DOCKER-BRIDGE", json!([name("oifname", "==", json!(bridge)), {"jump": {"target": "DOCKER"}}])),
            r("DOCKER", json!([{"match": {"op": "==", "left": {"payload": {"protocol": "ip", "field": "daddr"}}, "right": "172.18.0.3"}}, name("iifname", "!=", json!(bridge)), name("oifname", "==", json!(bridge)), {"accept": null}])),
            r("DOCKER", json!([name("iifname", "!=", json!(bridge)), name("oifname", "==", json!(bridge)), {"drop": null}])),
            {"chain": {"family": "ip", "table": "raw", "name": "PREROUTING", "type": "filter", "hook": "prerouting", "prio": -300, "policy": "accept"}},
            rule("ip", "raw", "PREROUTING", json!([{"match": {"op": "==", "left": {"payload": {"protocol": "ip", "field": "daddr"}}, "right": "172.18.0.3"}}, name("iifname", "!=", json!(bridge)), {"drop": null}])),
        ]))
    }

    fn prerouting(expr: Value) -> Ruleset {
        nft(json!([
            {"chain": {"family": "ip", "table": "raw", "name": "PREROUTING", "type": "filter", "hook": "prerouting", "prio": -300, "policy": "accept"}},
            rule("ip", "raw", "PREROUTING", expr),
        ]))
    }

    #[test]
    fn foreign_rules_conflict_only_when_they_can_match_exit_traffic() {
        let ufw = {
            let r = |chain, expr| rule("ip", "filter", chain, expr);
            nft(json!([
                chain("ip", "filter", "FORWARD", Some("forward"), "drop"),
                chain("ip", "filter", "ufw-before-forward", None, ""),
                r(
                    "FORWARD",
                    json!([{"jump": {"target": "ufw-before-forward"}}])
                ),
                r(
                    "ufw-before-forward",
                    json!([{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": ["related", "established"]}}, {"accept": null}])
                ),
            ]))
        };
        let firewalld = nft(json!([
            {"chain": {"family": "inet", "table": "firewalld", "name": "filter_FORWARD", "type": "filter", "hook": "forward", "prio": 10, "policy": "accept"}},
            rule("inet", "firewalld", "filter_FORWARD", json!([{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": ["established", "related"]}}, {"accept": null}])),
            rule("inet", "firewalld", "filter_FORWARD", json!([name("iifname", "==", json!("lo")), {"accept": null}])),
            rule("inet", "firewalld", "filter_FORWARD", json!([{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": "invalid"}}, {"drop": null}])),
            rule("inet", "firewalld", "filter_FORWARD", json!([{"reject": {"type": "icmpx", "expr": "admin-prohibited"}}])),
        ]));
        let invalid_only = nft(json!([
            {"chain": {"family": "inet", "table": "own", "name": "forward", "type": "filter", "hook": "forward", "prio": 0, "policy": "accept"}},
            rule("inet", "own", "forward", json!([{"match": {"op": "in", "left": {"ct": {"key": "state"}}, "right": "invalid"}}, {"drop": null}])),
            rule("inet", "own", "forward", json!([{"match": {"op": "==", "left": {"meta": {"key": "nfproto"}}, "right": "ipv6"}}, {"drop": null}])),
        ]));
        let handwritten = |expr| {
            nft(json!([
                {"chain": {"family": "inet", "table": "own", "name": "forward", "type": "filter", "hook": "forward", "prio": 0, "policy": "accept"}},
                rule("inet", "own", "forward", expr),
            ]))
        };
        let output_drop = nft(json!([
            {"chain": {"family": "inet", "table": "own", "name": "out", "type": "filter", "hook": "output", "prio": 0, "policy": "drop"}},
            rule("inet", "own", "out", json!([name("oifname", "==", json!("eth0")), {"accept": null}])),
        ]));
        let allowed_tun = nft(json!([
            chain("ip", "filter", "FORWARD", Some("forward"), "drop"),
            rule(
                "ip",
                "filter",
                "FORWARD",
                json!([name("iifname", "==", json!("tun*")), {"accept": null}])
            ),
            rule(
                "ip",
                "filter",
                "FORWARD",
                json!([name("oifname", "==", json!("tun*")), {"accept": null}])
            ),
        ]));
        let crossing = crossing();
        for (case, ruleset, expected) in [
            ("no rules", Ruleset::default(), Ok(())),
            ("docker, policy accept", docker("accept"), Ok(())),
            (
                "docker, FORWARD policy drop",
                docker("drop"),
                Err(Reason::FirewallConflict),
            ),
            (
                "ufw default forward drop",
                ufw,
                Err(Reason::FirewallConflict),
            ),
            (
                "firewalld zone reject",
                firewalld,
                Err(Reason::FirewallConflict),
            ),
            ("invalid and IPv6 drops", invalid_only, Ok(())),
            (
                "unconstrained drop",
                handwritten(json!([{"counter": null}, {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "drop out of the egress",
                handwritten(json!([name("oifname", "==", json!("eth0")), {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "drop into a wildcard tun",
                handwritten(json!([name("oifname", "==", json!("tun*")), {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "drop on another bridge",
                handwritten(
                    json!([name("iifname", "==", json!({"set": ["docker0", "br-*"]})), {"drop": null}]),
                ),
                Ok(()),
            ),
            (
                "drop off a named Mesh TUN",
                handwritten(json!([name("iifname", "==", json!("tun0")), {"drop": null}])),
                Ok(()),
            ),
            (
                "drop of an unused tun name",
                handwritten(json!([name("iifname", "==", json!("tun7")), {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "unjudged match",
                handwritten(
                    json!([{"match": {"op": "==", "left": {"payload": {"protocol": "tcp", "field": "dport"}}, "right": 25}}, {"reject": null}]),
                ),
                Err(Reason::FirewallConflict),
            ),
            (
                "verdict map",
                handwritten(
                    json!([{"vmap": {"key": {"meta": {"key": "l4proto"}}, "data": {"set": [["tcp", {"accept": null}], ["udp", {"drop": null}]]}}}]),
                ),
                Err(Reason::FirewallConflict),
            ),
            (
                "output policy drop",
                output_drop,
                Err(Reason::FirewallConflict),
            ),
            ("policy drop, exit TUN accepted", allowed_tun, Ok(())),
            (
                "raw prerouting drop of everything off a bridge",
                prerouting(json!([name("iifname", "!=", json!("docker0")), {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "prerouting drop of replies from the egress",
                prerouting(json!([name("iifname", "==", json!("eth0")), {"drop": null}])),
                Err(Reason::FirewallConflict),
            ),
            (
                "prerouting drop toward an Internet range",
                prerouting(
                    json!([{"match": {"op": "==", "left": {"payload": {"protocol": "ip", "field": "daddr"}}, "right": {"prefix": {"addr": "8.0.0.0", "len": 8}}}}, {"drop": null}]),
                ),
                Err(Reason::FirewallConflict),
            ),
            (
                "prerouting drop toward the Mesh pool",
                prerouting(
                    json!([{"match": {"op": "==", "left": {"payload": {"protocol": "ip", "field": "daddr"}}, "right": {"prefix": {"addr": "100.64.0.0", "len": 10}}}}, {"drop": null}]),
                ),
                Err(Reason::FirewallConflict),
            ),
            (
                "prerouting drop toward a container",
                prerouting(
                    json!([{"match": {"op": "==", "left": {"payload": {"protocol": "ip", "field": "daddr"}}, "right": {"set": ["172.18.0.3", {"prefix": {"addr": "10.0.0.0", "len": 8}}]}}}, {"drop": null}]),
                ),
                Ok(()),
            ),
        ] {
            assert_eq!(ruleset.judge(&crossing), expected, "{case}");
        }
        // Without an egress interface only locally sent errors cross.
        assert_eq!(docker("drop").judge(&Crossing::default()), Ok(()));
    }
}
