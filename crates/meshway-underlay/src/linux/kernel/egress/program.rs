use super::plan::Ranges;
use serde::{Deserialize, Serialize};

/// Interval elements each capture set holds (the kernel counts intervals):
/// a staged union of two plans.
const SET_SIZE: usize = super::plan::UNION_RANGES;
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Program {
    pub installation: String,
    pub mark: u32,
    /// The table hands managed IPv6 to the TUN (`managed6` while active,
    /// `sink6` always rejected). A table an earlier version created rejects
    /// all of `managed6`: its gates put both kinds there until it is
    /// recreated (at `down` or another boot).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub capture6: bool,
    /// The table keeps the replies of connections the host accepted from
    /// outside the Mesh on the underlay (chain `replies`). A table an
    /// earlier version created gains the chain in one transaction when its
    /// owner next opens it ([`Self::inbound_batch`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inbound: bool,
}
impl Program {
    pub fn table(&self) -> String {
        format!("meshway_egress_{}", self.installation)
    }
    pub fn create(&self) -> Value {
        let table = self.table();
        let mut items = vec![
            json!({"create": {"table": {"family": "inet", "name": table, "comment": format!("meshway-egress:{}:{:08x}", self.installation, self.mark)}}}),
        ];
        for (name, kind) in [
            ("managed4", "ipv4_addr"),
            ("managed6", "ipv6_addr"),
            ("mesh4", "ipv4_addr"),
            ("mesh6", "ipv6_addr"),
            ("sink6", "ipv6_addr"),
        ] {
            if name == "sink6" && !self.capture6 {
                continue;
            }
            items.push(json!({"add": {"set": {"family": "inet", "table": table, "name": name, "type": kind, "flags": ["interval"], "size": SET_SIZE}}}));
        }
        items.push(json!({"add": {"set": {"family": "inet", "table": table, "name": "active", "type": "ifname", "size": 1}}}));
        items.push(json!({"add": {"chain": {"family": "inet", "table": table, "name": "output", "type": "filter", "hook": "output", "prio": 300, "policy": "accept"}}}));
        let mut rules = vec![
            vec![
                compare(
                    "!=",
                    json!({"&": [{"meta": {"key": "mark"}}, self.mark]}),
                    json!(0),
                ),
                json!({"return": null}),
            ],
            vec![
                compare("==", json!({"meta": {"key": "oifname"}}), json!("lo")),
                json!({"return": null}),
            ],
            vec![
                compare(
                    "==",
                    json!({"fib": {"result": "type", "flags": ["daddr"]}}),
                    json!(2),
                ),
                json!({"return": null}),
            ],
        ];
        for source in [
            compare(
                "==",
                json!({"fib": {"result": "type", "flags": ["saddr"]}}),
                json!(2),
            ),
            compare(
                "==",
                json!({"payload": {"protocol": "ip6", "field": "saddr"}}),
                json!("::"),
            ),
        ] {
            rules.push(vec![
                compare(
                    "==",
                    json!({"payload": {"protocol": "ip6", "field": "hoplimit"}}),
                    json!(255),
                ),
                compare(
                    "==",
                    json!({"payload": {"protocol": "icmpv6", "field": "type"}}),
                    json!({"range": [133, 137]}),
                ),
                source,
                json!({"return": null}),
            ]);
        }
        for (protocol, family) in [("ip", 4), ("ip6", 6)] {
            let destination = json!({"payload": {"protocol": protocol, "field": "daddr"}});
            rules.push(vec![
                compare("==", destination.clone(), json!(format!("@mesh{family}"))),
                json!({"return": null}),
            ]);
            // Sunk IPv6 first: it never reaches the TUN, also where a staged
            // union holds it managed too.
            if family == 6 && self.capture6 {
                rules.push(vec![
                    compare("==", destination.clone(), json!("@sink6")),
                    compare("==", json!({"meta": {"key": "l4proto"}}), json!(6)),
                    json!({"reject": {"type": "tcp reset"}}),
                ]);
                rules.push(vec![
                    compare("==", destination.clone(), json!("@sink6")),
                    json!({"reject": {"type": "icmpx", "expr": "admin-prohibited"}}),
                ]);
            }
            if family == 4 || self.capture6 {
                rules.push(vec![
                    compare(
                        "==",
                        destination.clone(),
                        json!(format!("@managed{family}")),
                    ),
                    compare("==", json!({"meta": {"key": "oifname"}}), json!("@active")),
                    json!({"return": null}),
                ]);
            }
            // Rejected locally: a paused or sunk managed destination fails
            // at once, as an unreachable route would, and nothing is sent. A
            // TCP reset ends a connect immediately in both families.
            rules.push(vec![
                compare(
                    "==",
                    destination.clone(),
                    json!(format!("@managed{family}")),
                ),
                compare("==", json!({"meta": {"key": "l4proto"}}), json!(6)),
                json!({"reject": {"type": "tcp reset"}}),
            ]);
            rules.push(vec![
                compare("==", destination, json!(format!("@managed{family}"))),
                json!({"reject": {"type": "icmpx", "expr": "admin-prohibited"}}),
            ]);
        }
        for expr in rules {
            items.push(json!({"add": {"rule": {"family": "inet", "table": table, "chain": "output", "expr": expr}}}));
        }
        if self.inbound {
            items.extend(self.inbound_items());
        }
        json!({"nftables": items})
    }
    pub fn replace(&self, ranges: &Ranges, mesh: &Ranges, active: Option<&str>) -> Value {
        let mut commands = Vec::new();
        for (name, elements) in self.elements(ranges, mesh, active) {
            commands.push(
                json!({"flush": {"set": {"family": "inet", "table": self.table(), "name": name}}}),
            );
            if !elements.is_empty() {
                commands.push(json!({"add": {"element": {"family": "inet", "table": self.table(), "name": name, "elem": elements}}}));
            }
        }
        json!({"nftables": commands})
    }
    pub fn elements(
        &self,
        ranges: &Ranges,
        mesh: &Ranges,
        active: Option<&str>,
    ) -> Vec<(&'static str, Vec<Value>)> {
        fn range<T: std::fmt::Display + PartialEq>((start, end): &(T, T)) -> Value {
            if start == end {
                json!(start.to_string())
            } else {
                json!({"range": [start.to_string(), end.to_string()]})
            }
        }
        let mut sets = vec![
            ("managed4", ranges.v4.iter().map(range).collect()),
            if self.capture6 {
                ("managed6", ranges.v6.iter().map(range).collect())
            } else {
                // Without IPv6 capture every IPv6 range is rejected.
                (
                    "managed6",
                    meshway_rules::ranges::merge(
                        ranges.v6.iter().chain(&ranges.sink6).copied().collect(),
                    )
                    .iter()
                    .map(range)
                    .collect(),
                )
            },
            ("mesh4", mesh.v4.iter().map(range).collect()),
            ("mesh6", mesh.v6.iter().map(range).collect()),
            (
                "active",
                active.map(|name| vec![json!(name)]).unwrap_or_default(),
            ),
        ];
        if self.capture6 {
            sets.push(("sink6", ranges.sink6.iter().map(range).collect()));
        }
        sets
    }
    /// The inbound-reply exception, added to a table created without it.
    pub fn inbound_batch(&self) -> Value {
        json!({"nftables": self.inbound_items()})
    }
    /// The host's own packets of a connection another host opened (its
    /// conntrack reply direction), from an address outside the Mesh pools,
    /// carry the protected mark from `replies` on: routed again past the
    /// capture selector (the host's RPDB, the path the connection came on),
    /// they pass the gate of `output` too. Only the reply direction, so every
    /// connection the host opens is captured or rejected as before; a Mesh
    /// peer's connection (from a Mesh address) keeps its Mesh route. The mark
    /// bit is the one this Node already owns ([`Self::mark`]): nothing is
    /// written to conntrack, and the exit owner never selects this bit.
    fn inbound_items(&self) -> Vec<Value> {
        let table = self.table();
        let mut items = vec![
            json!({"add": {"chain": {"family": "inet", "table": table, "name": "replies", "type": "route", "hook": "output", "prio": -150, "policy": "accept"}}}),
        ];
        let mark = json!({"meta": {"key": "mark"}});
        for (protocol, family) in [("ip", 4), ("ip6", 6)] {
            let expr = vec![
                compare("==", json!({"ct": {"key": "direction"}}), json!("reply")),
                compare(
                    "!=",
                    json!({"ct": {"key": format!("{protocol} saddr"), "dir": "original"}}),
                    json!(format!("@mesh{family}")),
                ),
                json!({"mangle": {"key": mark.clone(), "value": {"|": [mark.clone(), self.mark]}}}),
            ];
            items.push(json!({"add": {"rule": {"family": "inet", "table": table, "chain": "replies", "expr": expr}}}));
        }
        items
    }
}
fn compare(op: &str, left: Value, right: Value) -> Value {
    json!({"match": {"op": op, "left": left, "right": right}})
}

#[cfg(test)]
mod unit {
    use super::*;
    use crate::linux::kernel::exit::objects::decode;

    fn objects(value: &Value) -> std::collections::BTreeMap<String, Value> {
        decode(&serde_json::to_vec(value).unwrap()).unwrap().objects
    }

    /// X19: the batch that upgrades a table of an earlier version adds
    /// exactly what a current table has beyond it, and it reads and writes
    /// only the packet's mark: conntrack is read, never written.
    #[test]
    fn the_inbound_batch_adds_exactly_the_reply_chain_of_a_current_table() {
        let earlier = Program {
            installation: "a".repeat(32),
            mark: 1 << 17,
            capture6: true,
            inbound: false,
        };
        let current = Program {
            inbound: true,
            ..earlier.clone()
        };
        let mut upgraded = objects(&earlier.create());
        let added = objects(&current.inbound_batch());
        assert_eq!(added.len(), 3, "{added:?}");
        for (key, value) in added {
            assert!(upgraded.insert(key, value).is_none());
        }
        assert_eq!(upgraded, objects(&current.create()));
        let batch = current.inbound_batch().to_string();
        assert!(batch.contains(r#""hook":"output""#) && batch.contains(r#""type":"route""#));
        assert!(!batch.contains(r#""mangle":{"key":{"ct""#));
        assert!(batch.contains(r#""right":"reply""#) && !batch.contains(r#""right":"original""#));
    }
}
