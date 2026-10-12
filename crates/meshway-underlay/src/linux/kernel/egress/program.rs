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
        ] {
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
            if family == 4 {
                rules.push(vec![
                    compare("==", destination.clone(), json!("@managed4")),
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
        vec![
            ("managed4", ranges.v4.iter().map(range).collect()),
            ("managed6", ranges.v6.iter().map(range).collect()),
            ("mesh4", mesh.v4.iter().map(range).collect()),
            ("mesh6", mesh.v6.iter().map(range).collect()),
            (
                "active",
                active.map(|name| vec![json!(name)]).unwrap_or_default(),
            ),
        ]
    }
}
fn compare(op: &str, left: Value, right: Value) -> Value {
    json!({"match": {"op": op, "left": left, "right": right}})
}
