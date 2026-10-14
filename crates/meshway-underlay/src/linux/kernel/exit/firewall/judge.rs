use super::{crossing::Crossing, interface::interface};
use meshway_model::exit::ExitUnavailableReason as Reason;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hook {
    Prerouting,
    Forward,
    Output,
    Postrouting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Truth {
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
pub(super) enum Interface<'a> {
    Named(&'a str),
    /// The exit TUN: the installed one by name, else a fresh kernel
    /// `tun<N>`, not a present interface.
    Exit,
    /// No interface (the input interface of a locally sent packet, the
    /// output interface before routing).
    Absent,
}

/// What an exit packet's IPv4 destination can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Destination {
    /// Forwarded out of the exit TUN: any Internet-classified address.
    Internet,
    /// A reply toward the exit TUN, or this host's error to it: an egress
    /// address (before the masquerade is undone) or a Mesh address.
    Reply,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Packet<'a> {
    hook: Hook,
    input: Interface<'a>,
    output: Interface<'a>,
    destination: Destination,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Match {
    Input(bool, Vec<String>),
    Output(bool, Vec<String>),
    /// `ct state` membership; exit traffic is new, established or related.
    State(bool, Vec<String>),
    /// `meta nfproto`: exit traffic is IPv4.
    Family(bool, Vec<String>),
    /// IPv4 destination membership (`ip daddr`, iptables `-d`).
    Destination(bool, Vec<ipnet::Ipv4Net>),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
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
pub(super) struct Rule {
    pub(super) matches: Vec<Match>,
    pub(super) verdict: Verdict,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Chain {
    pub(super) hook: Option<Hook>,
    pub(super) drops: bool,
    pub(super) rules: Vec<Rule>,
}

/// Chains keyed by (source, table, name); `source` separates nft families
/// from legacy iptables.
#[derive(Clone, Debug, Default)]
pub(super) struct Ruleset {
    pub(super) chains: BTreeMap<(String, String, String), Chain>,
}

const DEPTH: usize = 64;
/// Rules evaluated in one judgement, over every chain and packet. Each
/// (chain, packet) is walked once (later jumps reuse its end), so shared
/// subchains cannot multiply the work; a ruleset beyond this is unjudged.
const BUDGET: usize = 1 << 20;

type Key = (String, String, String);

/// Per-judgement state: the end of each (chain, packet) walked so far and
/// the rules evaluated.
struct Walk<'r> {
    ruleset: &'r Ruleset,
    crossing: &'r Crossing,
    ends: BTreeMap<(Key, usize), End>,
    work: usize,
}

impl Ruleset {
    /// The exit's packets on each judged hook: arriving from its TUN and
    /// from every egress interface (prerouting), forwarded out of and
    /// replies into its TUN through every egress interface, and this host's
    /// own ICMP errors sent to the TUN.
    pub(super) fn judge(&self, crossing: &Crossing) -> Result<(), Reason> {
        let mut packets = vec![Packet {
            hook: Hook::Output,
            input: Interface::Absent,
            output: Interface::Exit,
            destination: Destination::Reply,
        }];
        packets.push(Packet {
            hook: Hook::Prerouting,
            input: Interface::Exit,
            output: Interface::Absent,
            destination: Destination::Internet,
        });
        for egress in &crossing.egress {
            let egress = Interface::Named(egress);
            packets.push(Packet {
                hook: Hook::Prerouting,
                input: egress,
                output: Interface::Absent,
                destination: Destination::Reply,
            });
            for hook in [Hook::Forward, Hook::Postrouting] {
                packets.push(Packet {
                    hook,
                    input: Interface::Exit,
                    output: egress,
                    destination: Destination::Internet,
                });
                packets.push(Packet {
                    hook,
                    input: egress,
                    output: Interface::Exit,
                    destination: Destination::Reply,
                });
            }
        }
        let mut walk = Walk {
            ruleset: self,
            crossing,
            ends: BTreeMap::new(),
            work: 0,
        };
        for (key, chain) in &self.chains {
            let Some(hook) = chain.hook else { continue };
            for (index, packet) in packets.iter().enumerate() {
                if packet.hook != hook {
                    continue;
                }
                let end = walk.chain(key, chain, packet, index, 0)?;
                if chain.drops && end != End::Verdict {
                    return Err(Reason::FirewallConflict);
                }
            }
        }
        Ok(())
    }
}

impl Walk<'_> {
    /// How a packet that certainly reaches `chain` certainly leaves it; any
    /// rule that may block it is a conflict. `index` names the packet for
    /// the memo of ends.
    fn chain(
        &mut self,
        key: &Key,
        chain: &Chain,
        packet: &Packet<'_>,
        index: usize,
        depth: usize,
    ) -> Result<End, Reason> {
        if let Some(end) = self.ends.get(&(key.clone(), index)) {
            return Ok(*end);
        }
        let end = self.rules(key, chain, packet, index, depth)?;
        self.ends.insert((key.clone(), index), end);
        Ok(end)
    }

    fn rules(
        &mut self,
        key: &Key,
        chain: &Chain,
        packet: &Packet<'_>,
        index: usize,
        depth: usize,
    ) -> Result<End, Reason> {
        if depth > DEPTH {
            return Err(Reason::FirewallConflict);
        }
        let ruleset = self.ruleset;
        let target = |walk: &mut Self, name: &str| -> Result<End, Reason> {
            let key = (key.0.clone(), key.1.clone(), name.to_owned());
            let chain = ruleset.chains.get(&key).ok_or(Reason::FirewallConflict)?;
            walk.chain(&key, chain, packet, index, depth + 1)
        };
        for rule in &chain.rules {
            self.work += 1;
            if self.work > BUDGET {
                return Err(Reason::FirewallConflict);
            }
            let truth = rule.matches.iter().fold(Truth::Yes, |truth, matched| {
                truth.and(evaluate(matched, packet, self.crossing))
            });
            let certain = truth == Truth::Yes;
            match (&rule.verdict, truth) {
                (_, Truth::No) | (Verdict::Continue, _) => {}
                (Verdict::Block, _) => return Err(Reason::FirewallConflict),
                (Verdict::Accept, _) if certain => return Ok(End::Verdict),
                (Verdict::Return, _) if certain => return Ok(End::Return),
                (Verdict::Accept | Verdict::Return, _) => {}
                (Verdict::Jump(name), _) => {
                    if target(self, name)? == End::Verdict && certain {
                        return Ok(End::Verdict);
                    }
                }
                (Verdict::Goto(name), _) => {
                    let end = target(self, name)?;
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
                                target(self, name)?;
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

fn evaluate(matched: &Match, packet: &Packet<'_>, crossing: &Crossing) -> Truth {
    let member = |values: &[String], equal: bool, truth: &dyn Fn(&str) -> Truth| {
        let found = Truth::any(values.iter().map(|value| truth(value)));
        if equal { found } else { found.not() }
    };
    match matched {
        Match::Input(equal, names) => member(names, *equal, &|name| {
            interface(packet.input, name, crossing)
        }),
        Match::Output(equal, names) => member(names, *equal, &|name| {
            interface(packet.output, name, crossing)
        }),
        Match::Destination(equal, nets) => {
            let found = destination(packet.destination, nets, crossing);
            if *equal { found } else { found.not() }
        }
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

/// Whether a destination of class `class` lies in `nets`: Yes when every one
/// does, No when none can.
fn destination(class: Destination, nets: &[ipnet::Ipv4Net], crossing: &Crossing) -> Truth {
    let overlaps = |a: &ipnet::Ipv4Net, b: &ipnet::Ipv4Net| a.contains(b) || b.contains(a);
    let covered = |range: &ipnet::Ipv4Net| nets.iter().any(|net| net.contains(range));
    let ranges: Vec<ipnet::Ipv4Net> = match class {
        Destination::Internet => meshway_policy::internet_ranges()
            .iter()
            .filter_map(|range| match range {
                ipnet::IpNet::V4(range) => Some(*range),
                ipnet::IpNet::V6(_) => None,
            })
            .collect(),
        Destination::Reply => crossing
            .local
            .iter()
            .map(|ip| ipnet::Ipv4Net::from(*ip))
            .chain(crossing.mesh.iter().filter_map(|pool| match pool {
                ipnet::IpNet::V4(pool) => Some(*pool),
                ipnet::IpNet::V6(_) => None,
            }))
            .collect(),
    };
    if ranges.is_empty() {
        return Truth::Maybe;
    }
    if ranges.iter().all(covered) {
        Truth::Yes
    } else if ranges
        .iter()
        .any(|range| nets.iter().any(|net| overlaps(net, range)))
    {
        Truth::Maybe
    } else {
        Truth::No
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{chain, crossing, nft, rule};
    use super::*;
    use serde_json::{Value, json};

    /// Chains sharing subchains: each (chain, packet) is walked once, so 2^40
    /// paths cost 80 chain walks; a ruleset beyond the work budget is
    /// unjudged rather than walked.
    #[test]
    fn shared_subchains_are_walked_once_within_a_budget() {
        let mut items = vec![chain("ip", "filter", "FORWARD", Some("forward"), "accept")];
        items.push(rule(
            "ip",
            "filter",
            "FORWARD",
            json!([{"jump": {"target": "c0"}}]),
        ));
        for level in 0..40 {
            items.push(chain("ip", "filter", &format!("c{level}"), None, ""));
            for _ in 0..2 {
                items.push(rule(
                    "ip",
                    "filter",
                    &format!("c{level}"),
                    json!([{"jump": {"target": format!("c{}", level + 1)}}]),
                ));
            }
        }
        items.push(chain("ip", "filter", "c40", None, ""));
        let started = std::time::Instant::now();
        assert_eq!(nft(Value::Array(items)).judge(&crossing()), Ok(()));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let mut large = Ruleset::default();
        let forward = large
            .chains
            .entry(("ip".into(), "filter".into(), "FORWARD".into()))
            .or_default();
        forward.hook = Some(Hook::Forward);
        forward.rules = vec![
            Rule {
                matches: Vec::new(),
                verdict: Verdict::Continue,
            };
            BUDGET / 2 + 1
        ];
        assert_eq!(large.judge(&crossing()), Err(Reason::FirewallConflict));
    }
}
