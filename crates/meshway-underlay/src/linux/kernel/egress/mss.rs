//! TCP MSS clamp of the SYNs crossing this Node's TUNs, from the path
//! limits the packet owner knows. One owned table, `meshway_mss_<installation>`,
//! replaced whole in one nft transaction on every change and deleted with the
//! capture. It grants and blocks nothing: a lost or stale table only changes
//! the segment sizes TCP endpoints choose.
use ipnet::IpNet;
use meshway_tun::PathLimits;
use serde_json::{Value, json};
use std::{collections::BTreeSet, net::IpAddr};

/// Distinct clamp values one table holds; beyond it every entry takes the
/// smallest.
const MAX_VALUES: usize = 64;

/// What the table clamps for: the Mesh TUN, the exit TUN when this Node is an
/// exit, the Mesh pools (whose other addresses are never `other`), the TUN
/// MTU and the limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::linux::kernel) struct Clamp {
    pub mesh: String,
    pub exit: Option<String>,
    pub pools: Vec<IpNet>,
    pub mtu: usize,
    pub limits: PathLimits,
}

pub(in crate::linux::kernel) fn table(installation: &str) -> String {
    format!("meshway_mss_{installation}")
}

/// The MSS of a TCP segment within `limit` bytes, when below what the TUN MTU
/// already gives.
fn mss(limit: u16, ipv4: bool, mtu: usize) -> Option<u16> {
    let headers = if ipv4 { 40 } else { 60 };
    let value = usize::from(limit).checked_sub(headers)?;
    (value < mtu.saturating_sub(headers)).then(|| u16::try_from(value).ok())?
}

fn compare(op: &str, left: Value, right: Value) -> Value {
    json!({"match": {"op": op, "left": left, "right": right}})
}

/// The batch that leaves exactly `clamp`'s table, or none.
pub(super) fn batch(installation: &str, clamp: Option<&Clamp>) -> Value {
    let table = table(installation);
    let mut items = vec![
        json!({"add": {"table": {"family": "inet", "name": table}}}),
        json!({"delete": {"table": {"family": "inet", "name": table}}}),
    ];
    let Some(clamp) = clamp else {
        return json!({"nftables": items});
    };
    let entries: Vec<(IpAddr, u16)> = clamp
        .limits
        .peers
        .iter()
        .filter_map(|(address, limit)| Some((*address, mss(*limit, address.is_ipv4(), clamp.mtu)?)))
        .collect();
    // Destinations outside the Mesh (through an exit), by family: an IPv6
    // segment carries 20 more header bytes.
    let other = clamp
        .limits
        .other
        .and_then(|limit| mss(limit, true, clamp.mtu));
    let other6 = clamp
        .limits
        .other
        .and_then(|limit| mss(limit, false, clamp.mtu));
    let mut values: BTreeSet<u16> = entries.iter().map(|(_, value)| *value).collect();
    values.extend(other);
    values.extend(other6);
    let floor = values.first().copied();
    let value = |v: u16| {
        if values.len() > MAX_VALUES {
            floor.unwrap_or(v)
        } else {
            v
        }
    };
    if values.is_empty() {
        return json!({"nftables": items});
    }
    items.push(json!({"add": {"table": {"family": "inet", "name": table,
        "comment": format!("meshway-mss:{installation}")}}}));
    let chains: BTreeSet<u16> = values.iter().map(|v| value(*v)).collect();
    let option = json!({"tcp option": {"name": "maxseg", "field": "size"}});
    for v in &chains {
        let name = format!("m{v}");
        items.push(json!({"add": {"chain": {"family": "inet", "table": table, "name": name}}}));
        items.push(
            json!({"add": {"rule": {"family": "inet", "table": table, "chain": name,
            "expr": [compare(">", option.clone(), json!(v)),
                     {"mangle": {"key": option, "value": v}}]}}}),
        );
    }
    for (name, kind, ipv4) in [("peer4", "ipv4_addr", true), ("peer6", "ipv6_addr", false)] {
        let elem: Vec<Value> = entries
            .iter()
            .filter(|(address, _)| address.is_ipv4() == ipv4)
            .map(|(address, v)| {
                json!([address.to_string(), {"goto": {"target": format!("m{}", value(*v))}}])
            })
            .collect();
        let mut map = json!({"family": "inet", "table": table, "name": name, "type": kind,
            "map": "verdict"});
        if !elem.is_empty() {
            map["elem"] = json!(elem);
        }
        items.push(json!({"add": {"map": map}}));
    }
    let syn = compare(
        "==",
        json!({"&": [{"payload": {"protocol": "tcp", "field": "flags"}}, {"|": ["syn", "rst"]}]}),
        json!("syn"),
    );
    let pools = |ipv4: bool| -> Vec<Value> {
        clamp
            .pools
            .iter()
            .filter(|pool| matches!(pool, IpNet::V4(_)) == ipv4)
            .map(|net| json!({"prefix": {"addr": net.network().to_string(), "len": net.prefix_len()}}))
            .collect()
    };
    let address =
        |protocol: &str, field: &str| json!({"payload": {"protocol": protocol, "field": field}});
    // Locally sent SYNs by destination, received ones by source: what the
    // peer may send to this Node, and what this Node may send to it.
    for (chain, hook, interface, field) in [
        ("out", "output", "oifname", "daddr"),
        ("pre", "prerouting", "iifname", "saddr"),
    ] {
        items.push(
            json!({"add": {"chain": {"family": "inet", "table": table, "name": chain,
            "type": "filter", "hook": hook, "prio": -150, "policy": "accept"}}}),
        );
        let on = |name: &str| compare("==", json!({"meta": {"key": interface}}), json!(name));
        let mut rules = vec![
            vec![
                on(&clamp.mesh),
                syn.clone(),
                json!({"vmap": {"key": address("ip", field), "data": "@peer4"}}),
            ],
            vec![
                on(&clamp.mesh),
                syn.clone(),
                json!({"vmap": {"key": address("ip6", field), "data": "@peer6"}}),
            ],
        ];
        for (other, protocol, nfproto, ipv4) in
            [(other, "ip", "ipv4", true), (other6, "ip6", "ipv6", false)]
        {
            let Some(other) = other else { continue };
            let mut rule = vec![
                on(&clamp.mesh),
                syn.clone(),
                compare("==", json!({"meta": {"key": "nfproto"}}), json!(nfproto)),
            ];
            let pools = pools(ipv4);
            if !pools.is_empty() {
                rule.push(compare(
                    "!=",
                    address(protocol, field),
                    json!({"set": pools}),
                ));
            }
            rule.push(json!({"goto": {"target": format!("m{}", value(other))}}));
            rules.push(rule);
        }
        // An exit's forwarded SYN from a source: what the far end may send
        // back to that source.
        if hook == "prerouting"
            && let Some(exit) = &clamp.exit
        {
            rules.push(vec![
                on(exit),
                syn.clone(),
                json!({"vmap": {"key": address("ip", "saddr"), "data": "@peer4"}}),
            ]);
            rules.push(vec![
                on(exit),
                syn.clone(),
                json!({"vmap": {"key": address("ip6", "saddr"), "data": "@peer6"}}),
            ]);
        }
        for expr in rules {
            items.push(
                json!({"add": {"rule": {"family": "inet", "table": table, "chain": chain,
                "expr": expr}}}),
            );
        }
    }
    json!({"nftables": items})
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn values_below_the_tun_mtu_become_chains_and_map_entries() {
        let clamp = Clamp {
            mesh: "tun0".into(),
            exit: Some("tun1".into()),
            pools: vec!["100.64.0.0/10".parse().unwrap()],
            mtu: 1366,
            limits: PathLimits {
                peers: [
                    ("100.64.0.4".parse().unwrap(), 1330),
                    ("100.64.0.5".parse().unwrap(), 1386),
                    ("fd00::4".parse().unwrap(), 1330),
                ]
                .into(),
                other: Some(1300),
            },
        };
        let text = batch("ab", Some(&clamp)).to_string();
        // 1330 - 40, 1330 - 60, 1300 - 40, 1300 - 60 (IPv6 through an exit);
        // 1386 is above the TUN MTU.
        for chain in ["m1290", "m1270", "m1260", "m1240"] {
            assert!(text.contains(&format!("\"name\":\"{chain}\"")), "{chain}");
        }
        assert!(!text.contains("100.64.0.5"));
        assert!(text.contains("\"tun1\""));
        // No limit: only the removal.
        let empty = Clamp {
            limits: PathLimits::default(),
            ..clamp
        };
        assert_eq!(batch("ab", Some(&empty)), batch("ab", None));
    }
}
