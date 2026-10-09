use super::plan::Ranges;
use serde::{Deserialize, Serialize};
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
            items.push(json!({"add": {"set": {"family": "inet", "table": table, "name": name, "type": kind, "flags": ["interval"], "size": 8192}}}));
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
            rules.push(vec![
                compare("==", destination, json!(format!("@managed{family}"))),
                json!({"drop": null}),
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
        let nets = |values: &[ipnet::IpNet]| {
            values.iter().map(|net| json!({"prefix": {"addr": net.network().to_string(), "len": net.prefix_len()}})).collect()
        };
        vec![
            ("managed4", nets(&ranges.v4)),
            ("managed6", nets(&ranges.v6)),
            ("mesh4", nets(&mesh.v4)),
            ("mesh6", nets(&mesh.v6)),
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
