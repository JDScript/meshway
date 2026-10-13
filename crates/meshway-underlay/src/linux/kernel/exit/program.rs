//! Typed-input generation of the private IPv4 exit program.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const CAPACITY: usize = 128;
pub(super) const HISTORY: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Program {
    #[serde(default)]
    pub legacy: bool,
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

/// ICMPv4 destination unreachable and time exceeded: the RELATED errors an
/// owned flow's quoted forward packet may cause, at a router or this host.
fn error_type() -> Value {
    eq(
        json!({"payload": {"protocol": "icmp", "field": "type"}}),
        json!({"set": [3, 11]}),
    )
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
    /// A connection this host has not confirmed yet: the forward packet that
    /// caused a local error died before FORWARD (TTL, MTU), so it never
    /// received the owned mark.
    fn unconfirmed(&self) -> Value {
        // nft lists `ct status & confirmed == 0` in this negated-flag form.
        relation("!", ct("status"), json!(8))
    }
    fn open(&self) -> Value {
        relation(
            "!=",
            json!({"&": [ct("mark"), self.mark]}),
            json!("@closing"),
        )
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
        for (name, timed) in [
            ("flows", true),
            ("quarantine", false),
            ("echo_flows", true),
            ("echo_quarantine", false),
        ] {
            if self.legacy && name.starts_with("echo_") {
                continue;
            }
            let kind = if name.starts_with("echo_") {
                json!(["ipv4_addr", "ipv4_addr", "inet_service", "mark"])
            } else {
                json!([
                    "ipv4_addr",
                    "ipv4_addr",
                    "inet_proto",
                    "inet_service",
                    "inet_service"
                ])
            };
            let mut set = json!({"family": "inet", "table": self.filter(), "name": name, "type": kind, "size": HISTORY});
            if timed {
                set["flags"] = json!(["timeout"]);
                set["timeout"] = json!(1);
            }
            out.push(json!({"add": {"set": set}}));
        }
        if !self.legacy {
            out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "legacy_quarantine", "type": ["ipv4_addr", "ipv4_addr", "inet_proto", "inet_service", "inet_service"], "size": HISTORY}}}));
            // Old journals did not retain ICMP sequence. Their known wire
            // tuples remain broad negative tombstones; never infer a sequence.
        }
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "closing", "type": "mark", "size": 2}}}));
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "mesh", "type": "ipv4_addr", "flags": ["interval"], "elem": self.pools.iter().map(|pool| json!({"prefix": {"addr": pool.network().to_string(), "len": pool.prefix_len()}})).collect::<Vec<_>>()}}}));
        for (name, kind, hook, priority) in [
            ("raw", "filter", "prerouting", -300),
            ("route", "filter", "prerouting", -90),
            ("local", "route", "output", -150),
            ("input", "filter", "input", 0),
            ("forward", "filter", "forward", 0),
            ("final", "filter", "postrouting", 300),
        ] {
            out.push(json!({"add": {"chain": {"family": "inet", "table": self.filter(), "name": name, "type": kind, "hook": hook, "prio": priority, "policy": "accept"}}}));
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
        if !self.legacy {
            for chain in ["route", "forward", "final"] {
                out.push(self.rule(
                    chain,
                    vec![
                        self.owned(),
                        eq(original("reply"), json!("@legacy_quarantine")),
                        json!({"drop": null}),
                    ],
                ));
            }
        }
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
        // A router's error about an owned forward packet (conntrack RELATED,
        // reverse-translated) returns to the exit TUN like a reply.
        out.push(self.rule(
            "route",
            vec![
                self.owned(),
                direction("reply"),
                self.state(&["related"]),
                error_type(),
                active(),
                relation("!=", original("reply"), json!("@quarantine")),
                json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                json!({"return": null}),
            ],
        ));
        out.push(self.rule("route", vec![self.ingress(), json!({"drop": null})]));
        out.push(self.rule("route", vec![self.owned(), json!({"drop": null})]));
        // This host's own error (time exceeded, fragmentation needed) about an
        // owned or a not yet confirmed forward packet is rerouted to the exit
        // TUN. Every other local packet is the host's and passes untouched.
        out.push(self.rule(
            "local",
            vec![
                relation("!=", meta("nfproto"), json!("ipv4")),
                json!({"return": null}),
            ],
        ));
        for owner in [self.owned(), self.unconfirmed()] {
            out.push(self.rule(
                "local",
                vec![
                    owner,
                    self.open(),
                    direction("reply"),
                    self.state(&["related"]),
                    error_type(),
                    active(),
                    relation("!=", original("reply"), json!("@quarantine")),
                    json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                    json!({"return": null}),
                ],
            ));
        }
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
            let owners = if chain == "final" {
                vec![self.owned(), self.unconfirmed()]
            } else {
                vec![self.owned()]
            };
            for owner in owners {
                out.push(self.rule(
                    chain,
                    vec![
                        owner,
                        self.open(),
                        direction("reply"),
                        eq(meta("oif"), json!(self.interface)),
                        self.state(&["related"]),
                        error_type(),
                        active(),
                        relation("!=", original("reply"), json!("@quarantine")),
                        json!({"return": null}),
                    ],
                ));
            }
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
        let out: Vec<_> = out
            .into_iter()
            .flat_map(|rule| {
                if self.legacy {
                    vec![rule]
                } else {
                    sequence_rules(rule, self.mark)
                }
            })
            .collect();
        json!({"nftables": out})
    }
    pub fn close_gate(&self) -> Value {
        let mut commands = vec![
            self.element("add", "closing", vec![json!(0), json!(self.mark)]),
            json!({"flush": {"set": {"family": "inet", "table": self.filter(), "name": "flows"}}}),
        ];
        if !self.legacy {
            commands.push(json!({"flush": {"set": {"family": "inet", "table": self.filter(), "name": "echo_flows"}}}));
        }
        json!({"nftables": commands})
    }
    pub fn flow_set(&self, tuple: super::OriginalTuple) -> &'static str {
        if self.legacy { "flows" } else { tuple.set() }
    }
}

fn sequence_rules(rule: Value, mark: u32) -> Vec<Value> {
    let Some(expressions) = rule.pointer("/add/rule/expr").and_then(Value::as_array) else {
        return vec![rule];
    };
    if !expressions
        .iter()
        .any(|expr| expr.pointer("/match/right").and_then(Value::as_str) == Some("@flows"))
    {
        return vec![rule];
    }
    let mut ordinary = rule.clone();
    ordinary
        .pointer_mut("/add/rule/expr")
        .and_then(Value::as_array_mut)
        .expect("generated rule")
        .insert(0, relation("!=", ct("protocol"), json!(1)));
    // An error's echo identity is its quoted request's: after the 8-byte
    // ICMP header and an option-free 20-byte quoted IPv4 header.
    let related = expressions.contains(&error_type());
    let offset = if related { (8 + 20 + 4) * 8 } else { 32 };
    let mut echo = rule;
    let original = echo
        .pointer_mut("/add/rule/expr")
        .and_then(Value::as_array_mut)
        .expect("generated rule");
    let mut checks = vec![eq(ct("protocol"), json!(1))];
    if related {
        checks.push(eq(
            json!({"payload": {"base": "th", "offset": 64, "len": 8}}),
            json!(0x45),
        ));
    } else {
        checks.push(eq(
            json!({"payload": {"protocol": "icmp", "field": "type"}}),
            json!({"set": [0, 8]}),
        ));
    }
    let mut lookups = Vec::new();
    let mut actions = Vec::new();
    for mut expression in std::mem::take(original) {
        if let Some(name) = expression
            .pointer("/match/right")
            .and_then(Value::as_str)
            .filter(|name| matches!(*name, "@flows" | "@quarantine"))
            .map(str::to_owned)
        {
            let tuple = expression
                .pointer_mut("/match/left/concat")
                .and_then(Value::as_array_mut)
                .expect("generated tuple");
            tuple.remove(4);
            tuple.remove(2);
            tuple.push(meta("mark"));
            expression["match"]["right"] = json!(format!("@echo_{}", &name[1..]));
            lookups.push(expression);
        } else if expression.get("match").is_some() {
            checks.push(expression);
        } else if expression
            .pointer("/mangle/key/meta/key")
            .and_then(Value::as_str)
            != Some("mark")
        {
            actions.push(expression);
        }
    }
    // Pinned nft JSON lacks a 16-bit integer set type. A width-32 raw read
    // provides a typed mark register for sequence lookup inside this rule.
    // Reject unknown incoming mark bits; restore the owned routing bit before
    // every successful return. A failed lookup reaches the mandatory private
    // owner/ingress drop, so no RPDB or foreign hook observes the scratch value.
    checks.push(eq(meta("mark"), json!({"set": [0, mark]})));
    checks.push(json!({"mangle": {"key": meta("mark"), "value": {"payload": {"base": "th", "offset": offset, "len": 32}}}}));
    checks.push(json!({"mangle": {"key": meta("mark"), "value": {"&": [meta("mark"), 65535]}}}));
    checks.extend(lookups);
    let at = actions
        .iter()
        .position(|action| action.get("return").is_some())
        .unwrap_or(actions.len());
    actions.insert(at, json!({"mangle": {"key": meta("mark"), "value": mark}}));
    checks.extend(actions);
    *original = checks;
    vec![ordinary, echo]
}
