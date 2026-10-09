//! Typed-input generation of the private IPv4 exit program.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const CAPACITY: usize = 128;
pub(super) const HISTORY: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Program {
    pub installation: String,
    pub interface: u32,
    pub interface_name: String,
    pub mark: u32,
    pub pools: Vec<ipnet::Ipv4Net>,
}

fn ct(key: &str) -> Value {
    json!({"ct": {"key": key}})
}
fn meta(key: &str) -> Value {
    json!({"meta": {"key": key}})
}
fn packet(key: &str) -> Value {
    json!({"payload": {"protocol": "ip", "field": key}})
}
fn eq(left: Value, right: Value) -> Value {
    relation("==", left, right)
}
fn relation(op: &str, left: Value, right: Value) -> Value {
    json!({"match": {"op": op, "left": left, "right": right}})
}
fn original(direction: &str) -> Value {
    json!({"concat": [
        {"ct": {"key": "ip saddr", "dir": direction}},
        {"ct": {"key": "ip daddr", "dir": direction}},
        {"ct": {"key": "protocol"}},
        {"ct": {"key": "proto-src", "dir": direction}},
        {"ct": {"key": "proto-dst", "dir": direction}}
    ]})
}

impl Program {
    pub fn filter(&self) -> String {
        format!("meshway_exit_{}", self.installation)
    }
    pub fn nat(&self) -> String {
        format!("meshway_exit_nat_{}", self.installation)
    }
    pub fn tables(&self) -> [(&'static str, String); 2] {
        [("inet", self.filter()), ("ip", self.nat())]
    }
    fn ingress(&self) -> Value {
        eq(meta("iif"), json!(self.interface))
    }
    fn owned(&self) -> Value {
        eq(json!({"&": [ct("mark"), self.mark]}), json!(self.mark))
    }
    fn unowned(&self) -> Value {
        eq(json!({"&": [ct("mark"), self.mark]}), json!(0))
    }
    fn state(&self, states: &[&str]) -> Value {
        relation("in", ct("state"), json!({"set": states}))
    }
    fn rule(&self, chain: &str, expr: Vec<Value>) -> Value {
        json!({"add": {"rule": {"family": "inet", "table": self.filter(), "chain": chain, "expr": expr}}})
    }
    pub fn element(&self, operation: &str, name: &str, values: Vec<Value>) -> Value {
        json!({operation: {"element": {"family": "inet", "table": self.filter(), "name": name, "elem": values}}})
    }
    pub fn create(&self) -> Value {
        let mut out = Vec::new();
        for (family, name) in self.tables() {
            out.push(json!({"create": {"table": {"family": family, "name": name, "comment": format!("meshway-exit:{}:{}:{:08x}", self.installation, self.interface, self.mark)}}}));
        }
        for (name, timed) in [("flows", true), ("quarantine", false)] {
            let mut set = json!({"family": "inet", "table": self.filter(), "name": name, "type": ["ipv4_addr", "ipv4_addr", "inet_proto", "inet_service", "inet_service"], "size": HISTORY});
            if timed {
                set["flags"] = json!(["timeout"]);
                set["timeout"] = json!(1);
            }
            out.push(json!({"add": {"set": set}}));
        }
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "closing", "type": "mark", "size": 2}}}));
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "mesh", "type": "ipv4_addr", "flags": ["interval"], "elem": self.pools.iter().map(|pool| json!({"prefix": {"addr": pool.network().to_string(), "len": pool.prefix_len()}})).collect::<Vec<_>>()}}}));
        for (name, hook, priority) in [
            ("raw", "prerouting", -300),
            ("route", "prerouting", -90),
            ("input", "input", 0),
            ("forward", "forward", 0),
            ("final", "postrouting", 300),
        ] {
            out.push(json!({"add": {"chain": {"family": "inet", "table": self.filter(), "name": name, "type": "filter", "hook": hook, "prio": priority, "policy": "accept"}}}));
        }
        out.push(json!({"add": {"chain": {"family": "ip", "table": self.nat(), "name": "postrouting", "type": "nat", "hook": "postrouting", "prio": 100, "policy": "accept"}}}));
        out.push(self.rule(
            "raw",
            vec![
                relation("!=", meta("iif"), json!(self.interface)),
                eq(packet("saddr"), json!("@mesh")),
                relation("!=", packet("daddr"), json!("@mesh")),
                json!({"drop": null}),
            ],
        ));
        for chain in ["route", "input", "forward", "final"] {
            out.push(self.rule(
                chain,
                vec![
                    self.ingress(),
                    relation("!=", meta("nfproto"), json!("ipv4")),
                    json!({"drop": null}),
                ],
            ));
            out.push(self.rule(
                chain,
                vec![
                    relation("!=", meta("nfproto"), json!("ipv4")),
                    json!({"return": null}),
                ],
            ));
            for owner in [self.ingress(), self.owned()] {
                out.push(self.rule(
                    chain,
                    vec![
                        owner,
                        eq(json!({"&": [ct("mark"), self.mark]}), json!("@closing")),
                        json!({"drop": null}),
                    ],
                ));
            }
        }
        out.push(self.rule(
            "route",
            vec![
                self.ingress(),
                relation("!=", packet("saddr"), json!("@mesh")),
                json!({"drop": null}),
            ],
        ));
        out.push(self.rule(
            "route",
            vec![
                self.ingress(),
                eq(packet("daddr"), json!("@mesh")),
                json!({"drop": null}),
            ],
        ));
        for prefix in [
            "0.0.0.0/8",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "224.0.0.0/4",
            "240.0.0.0/4",
        ] {
            let (addr, len) = prefix.split_once('/').expect("constant");
            out.push(self.rule("route", vec![self.ingress(), eq(packet("daddr"), json!({"prefix": {"addr": addr, "len": len.parse::<u8>().expect("constant")}})), json!({"drop": null})]));
        }
        for (field, kinds) in [
            ("daddr", json!({"set": ["local", "broadcast", "multicast"]})),
            ("saddr", json!("local")),
        ] {
            out.push(self.rule(
                "route",
                vec![
                    self.ingress(),
                    eq(json!({"fib": {"result": "type", "flags": [field]}}), kinds),
                    json!({"drop": null}),
                ],
            ));
        }
        out.push(self.rule(
            "route",
            vec![
                self.ingress(),
                relation("in", ct("status"), json!(32)),
                json!({"drop": null}),
            ],
        ));
        let active = || eq(original("original"), json!("@flows"));
        let direction = |name| eq(ct("direction"), json!(name));
        // Route preparation grants no CT ownership. The only first assignment
        // occurs at FORWARD for a qualified NEW connection with a clear bit.
        for admission in [
            vec![self.state(&["new"]), self.unowned()],
            vec![self.state(&["new", "established", "related"]), self.owned()],
        ] {
            let mut expr = vec![self.ingress(), direction("original"), active()];
            expr.extend(admission);
            expr.extend([
                json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                json!({"return": null}),
            ]);
            out.push(self.rule("route", expr));
        }
        out.push(self.rule(
            "route",
            vec![
                self.owned(),
                direction("reply"),
                active(),
                relation("!=", original("reply"), json!("@quarantine")),
                json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                json!({"return": null}),
            ],
        ));
        out.push(self.rule("route", vec![self.ingress(), json!({"drop": null})]));
        out.push(self.rule("route", vec![self.owned(), json!({"drop": null})]));
        out.push(self.rule("input", vec![self.ingress(), json!({"drop": null})]));
        out.push(self.rule("input", vec![self.owned(), json!({"drop": null})]));
        out.push(self.rule(
            "forward",
            vec![
                self.ingress(),
                direction("original"),
                active(),
                self.state(&["new"]),
                self.unowned(),
                json!({"mangle": {"key": ct("mark"), "value": {"|": [ct("mark"), self.mark]}}}),
                json!({"return": null}),
            ],
        ));
        for chain in ["forward", "final"] {
            out.push(self.rule(
                chain,
                vec![
                    self.ingress(),
                    self.owned(),
                    direction("original"),
                    self.state(&["new", "established", "related"]),
                    active(),
                    relation("!=", original("reply"), json!("@quarantine")),
                    json!({"return": null}),
                ],
            ));
            out.push(self.rule(
                chain,
                vec![
                    self.owned(),
                    direction("reply"),
                    eq(meta("oif"), json!(self.interface)),
                    self.state(&["established", "related"]),
                    active(),
                    relation("!=", original("reply"), json!("@quarantine")),
                    json!({"return": null}),
                ],
            ));
            out.push(self.rule(chain, vec![self.ingress(), json!({"drop": null})]));
            out.push(self.rule(chain, vec![self.owned(), json!({"drop": null})]));
            out.push(self.rule(
                chain,
                vec![
                    eq(meta("oif"), json!(self.interface)),
                    json!({"drop": null}),
                ],
            ));
        }
        out.push(json!({"add": {"rule": {"family": "ip", "table": self.nat(), "chain": "postrouting", "expr": [self.owned(), direction("original"), self.ingress(), {"masquerade": null}]}}}));
        json!({"nftables": out})
    }
    pub fn close_gate(&self) -> Value {
        json!({"nftables": [self.element("add", "closing", vec![json!(0), json!(self.mark)]), {"flush": {"set": {"family": "inet", "table": self.filter(), "name": "flows"}}}]})
    }
}
