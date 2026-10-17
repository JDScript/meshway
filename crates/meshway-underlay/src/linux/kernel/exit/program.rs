//! Typed-input generation of the private exit program: IPv4, and IPv6
//! (NAT66) when the role carries it.
use meshway_model::exit::ExitFamily;
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
    /// Families the program carries, ascending; absent in a ledger written
    /// before families (IPv4 only).
    #[serde(
        default = "ipv4_only",
        skip_serializing_if = "meshway_configuration::is_default_exit_families"
    )]
    pub families: Vec<ExitFamily>,
    /// Mesh IPv6 pools; empty without IPv6.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pools6: Vec<ipnet::Ipv6Net>,
}

fn ipv4_only() -> Vec<ExitFamily> {
    vec![ExitFamily::Ipv4]
}

/// The per-family names and constants of one family's rules.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Family(ExitFamily);

impl Family {
    pub const V4: Self = Self(ExitFamily::Ipv4);
    pub const V6: Self = Self(ExitFamily::Ipv6);
    fn v6(self) -> bool {
        self.0 == ExitFamily::Ipv6
    }
    fn ip(self) -> &'static str {
        if self.v6() { "ip6" } else { "ip" }
    }
    fn address(self) -> &'static str {
        if self.v6() { "ipv6_addr" } else { "ipv4_addr" }
    }
    fn icmp(self) -> &'static str {
        if self.v6() { "icmpv6" } else { "icmp" }
    }
    fn icmp_number(self) -> u8 {
        if self.v6() { 58 } else { 1 }
    }
    /// The name of a set of this family: IPv6 sets end in `6`.
    pub fn set(self, name: &str) -> String {
        if self.v6() {
            format!("{name}6")
        } else {
            name.to_owned()
        }
    }
    fn of_set(name: &str) -> Self {
        if name.ends_with('6') {
            Self::V6
        } else {
            Self::V4
        }
    }
}

fn ct(key: &str) -> Value {
    json!({"ct": {"key": key}})
}
fn meta(key: &str) -> Value {
    json!({"meta": {"key": key}})
}
fn packet(family: Family, key: &str) -> Value {
    json!({"payload": {"protocol": family.ip(), "field": key}})
}
fn eq(left: Value, right: Value) -> Value {
    relation("==", left, right)
}
fn relation(op: &str, left: Value, right: Value) -> Value {
    json!({"match": {"op": op, "left": left, "right": right}})
}
fn original(family: Family, direction: &str) -> Value {
    json!({"concat": [
        {"ct": {"key": format!("{} saddr", family.ip()), "dir": direction}},
        {"ct": {"key": format!("{} daddr", family.ip()), "dir": direction}},
        {"ct": {"key": "protocol"}},
        {"ct": {"key": "proto-src", "dir": direction}},
        {"ct": {"key": "proto-dst", "dir": direction}}
    ]})
}

/// The RELATED errors an owned flow's quoted forward packet may cause, at a
/// router or this host: ICMPv4 destination unreachable and time exceeded;
/// ICMPv6 destination unreachable, packet too big and time exceeded (the
/// set the data plane parses; parameter problem is not passed).
fn error_type(family: Family) -> Value {
    let kinds: &[u8] = if family.v6() { &[1, 2, 3] } else { &[3, 11] };
    eq(
        json!({"payload": {"protocol": family.icmp(), "field": "type"}}),
        json!({"set": kinds}),
    )
}

impl Program {
    pub fn filter(&self) -> String {
        format!("meshway_exit_{}", self.installation)
    }
    pub fn nat(&self) -> String {
        format!("meshway_exit_nat_{}", self.installation)
    }
    pub fn nat6(&self) -> String {
        format!("meshway_exit_nat6_{}", self.installation)
    }
    /// The program's tables: the `inet` filter table, the `ip` NAT table and,
    /// when it carries IPv6, the `ip6` NAT table.
    pub fn tables(&self) -> Vec<(&'static str, String)> {
        let mut tables = vec![("inet", self.filter()), ("ip", self.nat())];
        if self.ipv6() {
            tables.push(("ip6", self.nat6()));
        }
        tables
    }
    /// The Mesh pools of every family the program carries.
    pub fn mesh_pools(&self) -> Vec<ipnet::IpNet> {
        self.pools
            .iter()
            .copied()
            .map(ipnet::IpNet::V4)
            .chain(self.pools6.iter().copied().map(ipnet::IpNet::V6))
            .collect()
    }
    pub fn ipv6(&self) -> bool {
        !self.legacy && self.families.contains(&ExitFamily::Ipv6)
    }
    /// The families the program's rules carry.
    pub(super) fn families(&self) -> Vec<Family> {
        if self.ipv6() {
            vec![Family::V4, Family::V6]
        } else {
            vec![Family::V4]
        }
    }
    /// The timed projection sets, `flows` and `echo_flows` of each family.
    pub(super) fn flow_sets(&self) -> Vec<String> {
        let mut sets = self.sets("flows");
        if !self.legacy {
            sets.extend(self.sets("echo_flows"));
        }
        sets
    }
    /// The quarantine sets of each family.
    pub(super) fn quarantine_sets(&self) -> Vec<String> {
        let mut sets = self.sets("quarantine");
        sets.extend(self.sets("echo_quarantine"));
        sets
    }
    /// Every per-family set of `name`.
    pub(super) fn sets(&self, name: &str) -> Vec<String> {
        self.families()
            .into_iter()
            .map(|family| family.set(name))
            .collect()
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
        for family in self.families() {
            let address = family.address();
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
                    json!([address, address, "inet_service", "mark"])
                } else {
                    json!([
                        address,
                        address,
                        "inet_proto",
                        "inet_service",
                        "inet_service"
                    ])
                };
                let mut set = json!({"family": "inet", "table": self.filter(), "name": family.set(name), "type": kind, "size": HISTORY});
                if timed {
                    set["flags"] = json!(["timeout"]);
                    set["timeout"] = json!(1);
                }
                out.push(json!({"add": {"set": set}}));
            }
        }
        if !self.legacy {
            out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "legacy_quarantine", "type": ["ipv4_addr", "ipv4_addr", "inet_proto", "inet_service", "inet_service"], "size": HISTORY}}}));
            // Old journals did not retain ICMP sequence. Their known wire
            // tuples remain broad negative tombstones; never infer a sequence.
        }
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "closing", "type": "mark", "size": 2}}}));
        out.push(json!({"add": {"set": {"family": "inet", "table": self.filter(), "name": "mesh", "type": "ipv4_addr", "flags": ["interval"], "elem": self.pools.iter().map(|pool| json!({"prefix": {"addr": pool.network().to_string(), "len": pool.prefix_len()}})).collect::<Vec<_>>()}}}));
        if self.ipv6() {
            let mut mesh = json!({"family": "inet", "table": self.filter(), "name": "mesh6", "type": "ipv6_addr", "flags": ["interval"]});
            if !self.pools6.is_empty() {
                mesh["elem"] = json!(self.pools6.iter().map(|pool| json!({"prefix": {"addr": pool.network().to_string(), "len": pool.prefix_len()}})).collect::<Vec<_>>());
            }
            out.push(json!({"add": {"set": mesh}}));
        }
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
        if self.ipv6() {
            out.push(json!({"add": {"chain": {"family": "ip6", "table": self.nat6(), "name": "postrouting", "type": "nat", "hook": "postrouting", "prio": 100, "policy": "accept"}}}));
        }
        let families = self.families();
        for family in &families {
            out.push(self.rule(
                "raw",
                vec![
                    relation("!=", meta("iif"), json!(self.interface)),
                    eq(
                        packet(*family, "saddr"),
                        json!(format!("@{}", family.set("mesh"))),
                    ),
                    relation(
                        "!=",
                        packet(*family, "daddr"),
                        json!(format!("@{}", family.set("mesh"))),
                    ),
                    json!({"drop": null}),
                ],
            ));
        }
        for chain in ["route", "input", "forward", "final"] {
            // A family the role does not carry: dropped from the exit TUN,
            // returned elsewhere. With both families every packet of the
            // `inet` hooks is carried.
            if !self.ipv6() {
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
            }
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
        for family in &families {
            let mesh = json!(format!("@{}", family.set("mesh")));
            // An address match also matches its own family only.
            out.push(self.rule(
                "route",
                vec![
                    self.ingress(),
                    relation("!=", packet(*family, "saddr"), mesh.clone()),
                    json!({"drop": null}),
                ],
            ));
            out.push(self.rule(
                "route",
                vec![
                    self.ingress(),
                    eq(packet(*family, "daddr"), mesh),
                    json!({"drop": null}),
                ],
            ));
            for prefix in meshway_policy::exit_exclusions()
                .iter()
                .filter(|net| net.addr().is_ipv6() == family.v6())
            {
                // nft lists a host prefix as the address itself.
                let destination = if prefix.prefix_len() == prefix.max_prefix_len() {
                    json!(prefix.addr().to_string())
                } else {
                    json!({"prefix": {"addr": prefix.addr().to_string(), "len": prefix.prefix_len()}})
                };
                out.push(self.rule(
                    "route",
                    vec![
                        self.ingress(),
                        eq(packet(*family, "daddr"), destination),
                        json!({"drop": null}),
                    ],
                ));
            }
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
        let active = |family: Family| {
            eq(
                original(family, "original"),
                json!(format!("@{}", family.set("flows"))),
            )
        };
        let quarantined = |family: Family| {
            relation(
                "!=",
                original(family, "reply"),
                json!(format!("@{}", family.set("quarantine"))),
            )
        };
        if !self.legacy {
            for chain in ["route", "forward", "final"] {
                out.push(self.rule(
                    chain,
                    vec![
                        self.owned(),
                        eq(original(Family::V4, "reply"), json!("@legacy_quarantine")),
                        json!({"drop": null}),
                    ],
                ));
            }
        }
        let direction = |name| eq(ct("direction"), json!(name));
        // Route preparation grants no CT ownership. The only first assignment
        // occurs at FORWARD for a qualified NEW connection with a clear bit.
        for family in &families {
            for admission in [
                vec![self.state(&["new"]), self.unowned()],
                vec![self.state(&["new", "established", "related"]), self.owned()],
            ] {
                let mut expr = vec![self.ingress(), direction("original"), active(*family)];
                expr.extend(admission);
                expr.extend([
                    json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                    json!({"return": null}),
                ]);
                out.push(self.rule("route", expr));
            }
        }
        for family in &families {
            out.push(self.rule(
                "route",
                vec![
                    self.owned(),
                    direction("reply"),
                    active(*family),
                    quarantined(*family),
                    json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                    json!({"return": null}),
                ],
            ));
        }
        // A router's error about an owned forward packet (conntrack RELATED,
        // reverse-translated) returns to the exit TUN like a reply.
        for family in &families {
            out.push(self.rule(
                "route",
                vec![
                    self.owned(),
                    direction("reply"),
                    self.state(&["related"]),
                    error_type(*family),
                    active(*family),
                    quarantined(*family),
                    json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                    json!({"return": null}),
                ],
            ));
        }
        out.push(self.rule("route", vec![self.ingress(), json!({"drop": null})]));
        out.push(self.rule("route", vec![self.owned(), json!({"drop": null})]));
        // This host's own error (time exceeded, fragmentation needed) about an
        // owned or a not yet confirmed forward packet is rerouted to the exit
        // TUN. Every other local packet is the host's and passes untouched.
        if !self.ipv6() {
            out.push(self.rule(
                "local",
                vec![
                    relation("!=", meta("nfproto"), json!("ipv4")),
                    json!({"return": null}),
                ],
            ));
        }
        for family in &families {
            for owner in [self.owned(), self.unconfirmed()] {
                out.push(self.rule(
                    "local",
                    vec![
                        owner,
                        self.open(),
                        direction("reply"),
                        self.state(&["related"]),
                        error_type(*family),
                        active(*family),
                        quarantined(*family),
                        json!({"mangle": {"key": meta("mark"), "value": {"|": [meta("mark"), self.mark]}}}),
                        json!({"return": null}),
                    ],
                ));
            }
        }
        out.push(self.rule("input", vec![self.ingress(), json!({"drop": null})]));
        out.push(self.rule("input", vec![self.owned(), json!({"drop": null})]));
        for family in &families {
            out.push(self.rule(
                "forward",
                vec![
                    self.ingress(),
                    direction("original"),
                    active(*family),
                    self.state(&["new"]),
                    self.unowned(),
                    json!({"mangle": {"key": ct("mark"), "value": {"|": [ct("mark"), self.mark]}}}),
                    json!({"return": null}),
                ],
            ));
        }
        for chain in ["forward", "final"] {
            for family in &families {
                out.push(self.rule(
                    chain,
                    vec![
                        self.ingress(),
                        self.owned(),
                        direction("original"),
                        self.state(&["new", "established", "related"]),
                        active(*family),
                        quarantined(*family),
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
                        active(*family),
                        quarantined(*family),
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
                            error_type(*family),
                            active(*family),
                            quarantined(*family),
                            json!({"return": null}),
                        ],
                    ));
                }
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
        if self.ipv6() {
            out.push(json!({"add": {"rule": {"family": "ip6", "table": self.nat6(), "chain": "postrouting", "expr": [self.owned(), direction("original"), self.ingress(), {"masquerade": null}]}}}));
        }
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
        let mut commands = vec![self.element("add", "closing", vec![json!(0), json!(self.mark)])];
        for name in self.sets("flows") {
            commands.push(
                json!({"flush": {"set": {"family": "inet", "table": self.filter(), "name": name}}}),
            );
        }
        if !self.legacy {
            for name in self.sets("echo_flows") {
                commands.push(json!({"flush": {"set": {"family": "inet", "table": self.filter(), "name": name}}}));
            }
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
    let Some(family) = expressions.iter().find_map(|expr| {
        expr.pointer("/match/right")
            .and_then(Value::as_str)
            .filter(|name| matches!(*name, "@flows" | "@flows6"))
            .map(Family::of_set)
    }) else {
        return vec![rule];
    };
    let icmp = family.icmp_number();
    let mut ordinary = rule.clone();
    ordinary
        .pointer_mut("/add/rule/expr")
        .and_then(Value::as_array_mut)
        .expect("generated rule")
        .insert(0, relation("!=", ct("protocol"), json!(icmp)));
    // An error's echo identity is its quoted request's: after the 8-byte
    // ICMP header and an option-free 20-byte quoted IPv4 header, or a
    // 40-byte quoted IPv6 header whose next header is ICMPv6 itself.
    let related = expressions.contains(&error_type(family));
    let quoted = if family.v6() { 40 } else { 20 };
    let offset = if related { (8 + quoted + 4) * 8 } else { 32 };
    let mut echo = rule;
    let original = echo
        .pointer_mut("/add/rule/expr")
        .and_then(Value::as_array_mut)
        .expect("generated rule");
    let mut checks = vec![eq(ct("protocol"), json!(icmp))];
    if related && family.v6() {
        // Quoted IPv6 version nibble, as nft lists it (a masked byte).
        checks.push(eq(
            json!({"&": [{"payload": {"base": "th", "offset": 64, "len": 8}}, 0xf0]}),
            json!(0x60),
        ));
        checks.push(eq(
            json!({"payload": {"base": "th", "offset": 112, "len": 8}}),
            json!(58),
        ));
    } else if related {
        checks.push(eq(
            json!({"payload": {"base": "th", "offset": 64, "len": 8}}),
            json!(0x45),
        ));
    } else {
        let kinds: &[u8] = if family.v6() { &[128, 129] } else { &[0, 8] };
        checks.push(eq(
            json!({"payload": {"protocol": family.icmp(), "field": "type"}}),
            json!({"set": kinds}),
        ));
    }
    let mut lookups = Vec::new();
    let mut actions = Vec::new();
    for mut expression in std::mem::take(original) {
        if let Some(name) = expression
            .pointer("/match/right")
            .and_then(Value::as_str)
            .filter(|name| matches!(*name, "@flows" | "@quarantine" | "@flows6" | "@quarantine6"))
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
