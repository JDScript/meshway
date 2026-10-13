//! Whether a foreign firewall can stop this exit's traffic. Only a rule that
//! can match that traffic is a conflict: a blocking verdict or `policy drop`
//! on the forward, output or postrouting hook whose interface constraints do
//! not exclude the exit TUN and the default-route interfaces. Rules confined
//! to other interfaces (Docker's bridges, say) are not. Never rewrite a host
//! firewall to obtain readiness.
use super::{
    ExitError,
    command::{Commands, Tool},
    host::{self, Failure},
};
use meshway_model::exit::ExitUnavailableReason as Reason;
use rtnetlink::packet_route::{
    AddressFamily,
    link::{LinkAttribute, LinkMessage},
    route::{RouteAddress, RouteAttribute, RouteMessage, RouteType},
};
use serde_json::Value;
use std::collections::BTreeMap;

/// The interfaces exit traffic crosses besides its own TUN.
#[derive(Clone, Debug, Default)]
pub(in crate::linux::kernel) struct Crossing {
    /// Interfaces of main-table IPv4 routes toward Internet-classified
    /// addresses: forwarded traffic leaves, replies arrive. Routes only to
    /// private networks (Docker bridges, say) are not exit paths to judge.
    pub egress: Vec<String>,
    /// Every interface present. The exit TUN is a new kernel `tun<N>`, none of these.
    pub present: Vec<String>,
}

impl Crossing {
    pub fn observe(links: &[LinkMessage], routes: &[RouteMessage]) -> Self {
        let names: BTreeMap<u32, String> = links
            .iter()
            .filter_map(|link| {
                link.attributes
                    .iter()
                    .find_map(|attribute| match attribute {
                        LinkAttribute::IfName(name) => Some((link.header.index, name.clone())),
                        _ => None,
                    })
            })
            .collect();
        let mut egress = Vec::new();
        for route in routes.iter().filter(|route| {
            route.header.address_family == AddressFamily::Inet
                && route.header.kind == RouteType::Unicast
                && super::super::plan::route_table(route) == 254
                && reaches_internet(route)
        }) {
            for attribute in &route.attributes {
                let indexes: Vec<u32> = match attribute {
                    RouteAttribute::Oif(index) => vec![*index],
                    RouteAttribute::MultiPath(hops) => {
                        hops.iter().map(|hop| hop.interface_index).collect()
                    }
                    _ => continue,
                };
                for index in indexes {
                    if let Some(name) = names.get(&index)
                        && !egress.contains(name)
                    {
                        egress.push(name.clone());
                    }
                }
            }
        }
        Self {
            egress,
            present: names.into_values().collect(),
        }
    }
}

/// Whether a route's destination holds an Internet-classified address: the
/// default route, or a connected public network.
fn reaches_internet(route: &RouteMessage) -> bool {
    let length = route.header.destination_prefix_length;
    let address = route
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            RouteAttribute::Destination(RouteAddress::Inet(address)) => Some(*address),
            _ => None,
        })
        .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
    let Ok(prefix) = ipnet::Ipv4Net::new(address, length).map(|net| ipnet::IpNet::V4(net.trunc()))
    else {
        return false;
    };
    meshway_policy::internet_ranges()
        .iter()
        .any(|range| range.contains(&prefix) || prefix.contains(range))
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hook {
    Forward,
    Output,
    Postrouting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth {
    Yes,
    No,
    Maybe,
}

impl Truth {
    fn not(self) -> Self {
        match self {
            Self::Yes => Self::No,
            Self::No => Self::Yes,
            Self::Maybe => Self::Maybe,
        }
    }
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Maybe,
        }
    }
    fn any(values: impl IntoIterator<Item = Self>) -> Self {
        let mut result = Self::No;
        for value in values {
            match value {
                Self::Yes => return Self::Yes,
                Self::Maybe => result = Self::Maybe,
                Self::No => {}
            }
        }
        result
    }
}

#[derive(Clone, Copy, Debug)]
enum Interface<'a> {
    Named(&'a str),
    /// The exit TUN: a fresh kernel `tun<N>`, not a present interface.
    Exit,
    /// No interface (the input interface of a locally sent packet).
    Absent,
}

#[derive(Clone, Copy, Debug)]
struct Packet<'a> {
    hook: Hook,
    input: Interface<'a>,
    output: Interface<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Match {
    Input(bool, Vec<String>),
    Output(bool, Vec<String>),
    /// `ct state` membership; exit traffic is new, established or related.
    State(bool, Vec<String>),
    /// `meta nfproto`: exit traffic is IPv4.
    Family(bool, Vec<String>),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Continue,
    Accept,
    Return,
    Block,
    Jump(String),
    Goto(String),
    /// A verdict map keyed on an unjudged value: any one of these.
    Map(Vec<Verdict>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    matches: Vec<Match>,
    verdict: Verdict,
}

#[derive(Clone, Debug, Default)]
struct Chain {
    hook: Option<Hook>,
    drops: bool,
    rules: Vec<Rule>,
}

/// Chains keyed by (source, table, name); `source` separates nft families
/// from legacy iptables.
#[derive(Clone, Debug, Default)]
pub(super) struct Ruleset {
    chains: BTreeMap<(String, String, String), Chain>,
}

const DEPTH: usize = 64;

impl Ruleset {
    fn from_nft(objects: &[host::Object]) -> Self {
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

    /// The exit's packets on each judged hook: forwarded out of and replies
    /// into its TUN through every egress interface, and this host's own ICMP
    /// errors sent to the TUN.
    fn judge(&self, crossing: &Crossing) -> Result<(), Reason> {
        let mut packets = vec![Packet {
            hook: Hook::Output,
            input: Interface::Absent,
            output: Interface::Exit,
        }];
        for egress in &crossing.egress {
            let egress = Interface::Named(egress);
            for hook in [Hook::Forward, Hook::Postrouting] {
                packets.push(Packet {
                    hook,
                    input: Interface::Exit,
                    output: egress,
                });
                packets.push(Packet {
                    hook,
                    input: egress,
                    output: Interface::Exit,
                });
            }
        }
        for ((source, table, _), chain) in &self.chains {
            let Some(hook) = chain.hook else { continue };
            for packet in packets.iter().filter(|packet| packet.hook == hook) {
                let end = self.walk(source, table, chain, packet, &crossing.present, 0)?;
                if chain.drops && end != End::Verdict {
                    return Err(Reason::FirewallConflict);
                }
            }
        }
        Ok(())
    }

    /// How a packet that certainly reaches `chain` certainly leaves it; any
    /// rule that may block it is a conflict.
    fn walk(
        &self,
        source: &str,
        table: &str,
        chain: &Chain,
        packet: &Packet<'_>,
        present: &[String],
        depth: usize,
    ) -> Result<End, Reason> {
        if depth > DEPTH {
            return Err(Reason::FirewallConflict);
        }
        let target = |name: &str| -> Result<End, Reason> {
            let chain = self
                .chains
                .get(&(source.to_owned(), table.to_owned(), name.to_owned()))
                .ok_or(Reason::FirewallConflict)?;
            self.walk(source, table, chain, packet, present, depth + 1)
        };
        for rule in &chain.rules {
            let truth = rule.matches.iter().fold(Truth::Yes, |truth, matched| {
                truth.and(evaluate(matched, packet, present))
            });
            let certain = truth == Truth::Yes;
            match (&rule.verdict, truth) {
                (_, Truth::No) | (Verdict::Continue, _) => {}
                (Verdict::Block, _) => return Err(Reason::FirewallConflict),
                (Verdict::Accept, _) if certain => return Ok(End::Verdict),
                (Verdict::Return, _) if certain => return Ok(End::Return),
                (Verdict::Accept | Verdict::Return, _) => {}
                (Verdict::Jump(name), _) => {
                    if target(name)? == End::Verdict && certain {
                        return Ok(End::Verdict);
                    }
                }
                (Verdict::Goto(name), _) => {
                    let end = target(name)?;
                    if certain {
                        return Ok(if end == End::Verdict {
                            End::Verdict
                        } else {
                            End::Return
                        });
                    }
                }
                (Verdict::Map(verdicts), _) => {
                    for verdict in verdicts {
                        match verdict {
                            Verdict::Block => return Err(Reason::FirewallConflict),
                            Verdict::Jump(name) | Verdict::Goto(name) => {
                                target(name)?;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(End::Fall)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Accepted (or dropped) in this or a called chain: no later rule applies.
    Verdict,
    Return,
    Fall,
}

fn evaluate(matched: &Match, packet: &Packet<'_>, present: &[String]) -> Truth {
    let member = |values: &[String], equal: bool, truth: &dyn Fn(&str) -> Truth| {
        let found = Truth::any(values.iter().map(|value| truth(value)));
        if equal { found } else { found.not() }
    };
    match matched {
        Match::Input(equal, names) => member(names, *equal, &|name| {
            interface(packet.input, name, present)
        }),
        Match::Output(equal, names) => member(names, *equal, &|name| {
            interface(packet.output, name, present)
        }),
        Match::State(equal, states) => {
            let ours = ["new", "established", "related"];
            let covered = ours
                .iter()
                .filter(|state| states.iter().any(|value| value == *state))
                .count();
            let found = match covered {
                0 => Truth::No,
                3 => Truth::Yes,
                _ => Truth::Maybe,
            };
            if *equal { found } else { found.not() }
        }
        Match::Family(equal, families) => member(families, *equal, &|family| match family {
            "ipv4" => Truth::Yes,
            "ipv6" | "arp" | "bridge" => Truth::No,
            _ => Truth::Maybe,
        }),
        Match::Unknown => Truth::Maybe,
    }
}

/// Whether interface `actual` has the name `pattern`; a trailing `*` (nft)
/// or `+` (iptables) matches any suffix.
fn interface(actual: Interface<'_>, pattern: &str, present: &[String]) -> Truth {
    let (prefix, wildcard) = match pattern
        .strip_suffix('*')
        .or_else(|| pattern.strip_suffix('+'))
    {
        Some(prefix) => (prefix, true),
        None => (pattern, false),
    };
    let fresh = |name: &str| {
        name.strip_prefix("tun")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
            && !present.iter().any(|known| known == name)
    };
    let truth = |value: bool| if value { Truth::Yes } else { Truth::No };
    match actual {
        Interface::Named(name) if wildcard => truth(name.starts_with(prefix)),
        Interface::Named(name) => truth(name == pattern),
        Interface::Absent if wildcard => truth(prefix.is_empty()),
        Interface::Absent => truth(pattern.is_empty()),
        Interface::Exit if wildcard => {
            if "tun".starts_with(prefix) {
                Truth::Yes
            } else if prefix
                .strip_prefix("tun")
                .is_some_and(|number| number.bytes().all(|b| b.is_ascii_digit()))
            {
                Truth::Maybe
            } else {
                Truth::No
            }
        }
        Interface::Exit if fresh(pattern) => Truth::Maybe,
        Interface::Exit => Truth::No,
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

fn legacy_blocks(target: &str) -> bool {
    matches!(
        target,
        "DROP" | "REJECT" | "QUEUE" | "NFQUEUE" | "TPROXY" | "TEE"
    )
}

/// `iptables-legacy-save` output: its chains, and whether any rule reads or
/// writes the packet or connection mark or conntrack handling.
#[derive(Debug, Default)]
pub(super) struct Legacy {
    ruleset: Ruleset,
    pub marks: bool,
}

impl Legacy {
    pub fn parse(text: &str) -> Result<Self, Failure> {
        let mut legacy = Self::default();
        let mut table: Option<String> = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('*') {
                table = Some(name.to_owned());
                continue;
            }
            if line == "COMMIT" {
                table = None;
                continue;
            }
            let table = table.as_deref().ok_or(Failure::Malformed)?;
            let key = |chain: &str| ("legacy".to_owned(), table.to_owned(), chain.to_owned());
            if let Some(declaration) = line.strip_prefix(':') {
                let mut fields = declaration.split_whitespace();
                let name = fields.next().ok_or(Failure::Malformed)?;
                let policy = fields.next().ok_or(Failure::Malformed)?;
                let chain = legacy.ruleset.chains.entry(key(name)).or_default();
                chain.hook = match (table, name) {
                    ("filter" | "mangle" | "security", "FORWARD") => Some(Hook::Forward),
                    ("filter" | "mangle" | "raw" | "nat" | "security", "OUTPUT") => {
                        Some(Hook::Output)
                    }
                    ("mangle" | "nat", "POSTROUTING") => Some(Hook::Postrouting),
                    _ => None,
                };
                chain.drops = policy == "DROP";
                continue;
            }
            let tokens = tokenize(line)?;
            let (Some("-A"), Some(chain)) = (
                tokens.first().map(String::as_str),
                tokens.get(1).map(String::as_str),
            ) else {
                return Err(Failure::Malformed);
            };
            let declared = |name: &str| legacy.ruleset.chains.contains_key(&key(name));
            let (rule, marks) = legacy_rule(&tokens[2..], &declared)?;
            legacy.marks |= marks;
            legacy
                .ruleset
                .chains
                .entry(key(chain))
                .or_default()
                .rules
                .push(rule);
        }
        if table.is_some() || legacy.ruleset.chains.len() > 16384 {
            return Err(Failure::Malformed);
        }
        Ok(legacy)
    }
}

fn tokenize(line: &str) -> Result<Vec<String>, Failure> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => quoted = !quoted,
            '\\' if quoted => current.push(chars.next().ok_or(Failure::Malformed)?),
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            character => current.push(character),
        }
    }
    if quoted || tokens.len() > 512 {
        return Err(Failure::Malformed);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// One `-A` rule; `declared` names this table's chains, any other target is
/// an extension.
fn legacy_rule(
    tokens: &[String],
    declared: &dyn Fn(&str) -> bool,
) -> Result<(Rule, bool), Failure> {
    let mut matches = Vec::new();
    let mut verdict = Verdict::Continue;
    let mut marks = false;
    let mut negated = false;
    let mut index = 0;
    let value = |index: usize| tokens.get(index).cloned().ok_or(Failure::Malformed);
    while index < tokens.len() {
        let token = tokens[index].as_str();
        match token {
            "!" => {
                negated = true;
                index += 1;
                continue;
            }
            "-i" | "--in-interface" => {
                matches.push(Match::Input(!negated, vec![value(index + 1)?]))
            }
            "-o" | "--out-interface" => {
                matches.push(Match::Output(!negated, vec![value(index + 1)?]))
            }
            "--ctstate" | "--state" => matches.push(Match::State(
                !negated,
                value(index + 1)?
                    .split(',')
                    .map(str::to_ascii_lowercase)
                    .collect(),
            )),
            "-m" | "--match" => {
                marks |= matches!(value(index + 1)?.as_str(), "mark" | "connmark");
            }
            "-j" | "--jump" | "-g" | "--goto" => {
                let target = value(index + 1)?;
                marks |= matches!(
                    target.as_str(),
                    "MARK" | "CONNMARK" | "CT" | "NOTRACK" | "TPROXY"
                );
                verdict = match target.as_str() {
                    "ACCEPT" => Verdict::Accept,
                    "RETURN" => Verdict::Return,
                    target if legacy_blocks(target) => Verdict::Block,
                    target if !declared(target) => Verdict::Continue,
                    target if matches!(token, "-g" | "--goto") => Verdict::Goto(target.to_owned()),
                    target => Verdict::Jump(target.to_owned()),
                };
            }
            "--comment" => {}
            "-p" | "--protocol" | "-s" | "--source" | "-d" | "--destination" | "-f"
            | "--fragment" => matches.push(Match::Unknown),
            option if option.starts_with('-') && !matches!(option, "-c") => {
                matches.push(Match::Unknown)
            }
            _ => {}
        }
        negated = false;
        index += 1;
    }
    Ok((Rule { matches, verdict }, marks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn crossing() -> Crossing {
        Crossing {
            egress: vec!["eth0".into()],
            present: ["lo", "eth0", "docker0", "br-35ac0d23089d", "tun0"]
                .map(String::from)
                .to_vec(),
        }
    }

    fn nft(items: Value) -> Ruleset {
        let document = json!({"nftables": items});
        let objects = host::decode_ruleset(&serde_json::to_vec(&document).unwrap(), &[]).unwrap();
        Ruleset::from_nft(&objects)
    }

    fn chain(family: &str, table: &str, name: &str, hook: Option<&str>, policy: &str) -> Value {
        let mut chain = json!({"family": family, "table": table, "name": name});
        if let Some(hook) = hook {
            chain["type"] = json!("filter");
            chain["hook"] = json!(hook);
            chain["prio"] = json!(0);
            chain["policy"] = json!(policy);
        }
        json!({"chain": chain})
    }

    fn rule(family: &str, table: &str, chain: &str, expr: Value) -> Value {
        json!({"rule": {"family": family, "table": table, "chain": chain, "expr": expr}})
    }

    fn name(key: &str, op: &str, right: Value) -> Value {
        json!({"match": {"op": op, "left": {"meta": {"key": key}}, "right": right}})
    }

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
            rule("ip", "raw", "PREROUTING", json!([name("iifname", "!=", json!(bridge)), {"drop": null}])),
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
        ] {
            assert_eq!(ruleset.judge(&crossing), expected, "{case}");
        }
        // Without an egress interface only locally sent errors cross.
        assert_eq!(docker("drop").judge(&Crossing::default()), Ok(()));
    }

    #[test]
    fn crossing_holds_the_interfaces_toward_internet_addresses() {
        let link = |index, name: &str| {
            let mut link = LinkMessage::default();
            link.header.index = index;
            link.attributes.push(LinkAttribute::IfName(name.into()));
            link
        };
        let route = |destination: &str, length, oif, table| {
            let mut route = RouteMessage::default();
            route.header.address_family = AddressFamily::Inet;
            route.header.kind = RouteType::Unicast;
            route.header.destination_prefix_length = length;
            route.attributes.push(RouteAttribute::Table(table));
            if length > 0 {
                route
                    .attributes
                    .push(RouteAttribute::Destination(RouteAddress::Inet(
                        destination.parse().unwrap(),
                    )));
            }
            route.attributes.push(RouteAttribute::Oif(oif));
            route
        };
        let links = [
            link(1, "lo"),
            link(2, "eth0"),
            link(3, "docker0"),
            link(4, "ext"),
            link(5, "lan"),
            link(6, "tun0"),
        ];
        let routes = [
            route("0.0.0.0", 0, 2, 254),
            route("104.194.68.0", 22, 2, 254),
            route("172.17.0.0", 16, 3, 254),
            route("11.77.3.0", 29, 4, 254),
            route("192.168.1.0", 24, 5, 254),
            route("0.0.0.0", 0, 6, 1000),
        ];
        let crossing = Crossing::observe(&links, &routes);
        assert_eq!(crossing.egress, ["eth0", "ext"]);
        assert_eq!(crossing.present.len(), 6);
        // Without a default route the connected public network remains.
        assert_eq!(Crossing::observe(&links, &routes[2..]).egress, ["ext"]);
    }

    #[test]
    fn exit_tun_names_are_fresh_kernel_tun_names() {
        let present = ["tun0".to_owned(), "eth0".to_owned()];
        for (pattern, expected) in [
            ("tun*", Truth::Yes),
            ("t+", Truth::Yes),
            ("tun1*", Truth::Maybe),
            ("tun1", Truth::Maybe),
            ("tun0", Truth::No),
            ("tunnel*", Truth::No),
            ("eth0", Truth::No),
            ("*", Truth::Yes),
        ] {
            assert_eq!(
                interface(Interface::Exit, pattern, &present),
                expected,
                "{pattern}"
            );
        }
        assert_eq!(
            interface(Interface::Named("br-1"), "br-*", &present),
            Truth::Yes
        );
        assert_eq!(interface(Interface::Absent, "eth0", &present), Truth::No);
    }

    #[test]
    fn legacy_chains_are_judged_by_content() {
        let docker = "*nat\n:PREROUTING ACCEPT [0:0]\n:POSTROUTING ACCEPT [0:0]\n:DOCKER - [0:0]\n\
            -A POSTROUTING -s 172.17.0.0/16 ! -o docker0 -j MASQUERADE\nCOMMIT\n\
            *filter\n:INPUT ACCEPT [0:0]\n:FORWARD ACCEPT [0:0]\n:OUTPUT ACCEPT [0:0]\n\
            :DOCKER - [0:0]\n:DOCKER-ISOLATION-STAGE-1 - [0:0]\n:DOCKER-ISOLATION-STAGE-2 - [0:0]\n:DOCKER-USER - [0:0]\n\
            -A FORWARD -j DOCKER-USER\n-A FORWARD -j DOCKER-ISOLATION-STAGE-1\n\
            -A FORWARD -o docker0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT\n\
            -A FORWARD -o docker0 -j DOCKER\n-A FORWARD -i docker0 ! -o docker0 -j ACCEPT\n\
            -A DOCKER-ISOLATION-STAGE-1 -i docker0 ! -o docker0 -j DOCKER-ISOLATION-STAGE-2\n\
            -A DOCKER-ISOLATION-STAGE-1 -j RETURN\n-A DOCKER-ISOLATION-STAGE-2 -o docker0 -j DROP\n\
            -A DOCKER-ISOLATION-STAGE-2 -j RETURN\n-A DOCKER-USER -j RETURN\n\
            -A DOCKER -d 172.17.0.2/32 ! -i docker0 -o docker0 -p tcp -m tcp --dport 80 -j ACCEPT\nCOMMIT\n";
        let legacy = Legacy::parse(docker).unwrap();
        assert!(!legacy.marks);
        assert_eq!(legacy.ruleset.judge(&crossing()), Ok(()));
        let dropping = Legacy::parse(&docker.replace(":FORWARD ACCEPT", ":FORWARD DROP")).unwrap();
        assert_eq!(
            dropping.ruleset.judge(&crossing()),
            Err(Reason::FirewallConflict)
        );
        let ufw = "*filter\n:FORWARD DROP [0:0]\n:ufw-user-forward - [0:0]\n\
            -A FORWARD -j ufw-user-forward\n-A ufw-user-forward -i eth0 -o eth0 -j ACCEPT\n\
            -A ufw-user-forward -m comment --comment \"a \\\"quoted\\\" note\" -j RETURN\nCOMMIT\n";
        assert_eq!(
            Legacy::parse(ufw).unwrap().ruleset.judge(&crossing()),
            Err(Reason::FirewallConflict)
        );
        let marking = "*mangle\n:PREROUTING ACCEPT [0:0]\n-A PREROUTING -i eth1 -j MARK --set-xmark 0x1/0xffffffff\nCOMMIT\n";
        assert!(Legacy::parse(marking).unwrap().marks);
        for malformed in [
            "-A FORWARD -j DROP\n",
            "*filter\n-A FORWARD -i\nCOMMIT\n",
            "*filter\n",
        ] {
            assert!(Legacy::parse(malformed).is_err(), "{malformed:?}");
        }
    }
}
