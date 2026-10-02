use super::*;
use ipnet::IpNet;
use std::{collections::BTreeMap, net::IpAddr};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct PrefixNode {
    children: [Option<u32>; 2],
    rules: Vec<u16>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct DomainNode {
    children: BTreeMap<String, u32>,
    rules: Vec<u16>,
}
/// Shared matcher index: bit tries for IP, reverse-label trie for DNS. Runtime
/// lookups do not parse CIDRs or linearly scan thousands of destination ranges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DestinationIndex {
    ipv4: Vec<PrefixNode>,
    ipv6: Vec<PrefixNode>,
    domains: Vec<DomainNode>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlattenedPrefix {
    pub network: String,
    pub rule: u16,
}
enum Partition {
    Leaf(Option<u16>),
    Branch(Box<Partition>, Box<Partition>),
}
fn fully_covered(p: &Partition) -> bool {
    match p {
        Partition::Leaf(rule) => rule.is_some(),
        Partition::Branch(a, b) => fully_covered(a) && fully_covered(b),
    }
}
fn partition(
    nodes: &[PrefixNode],
    node: Option<usize>,
    inherited: Option<u16>,
    enabled: &BTreeSet<u16>,
) -> Partition {
    let Some(node) = node else {
        return Partition::Leaf(inherited);
    };
    let winner = nodes[node]
        .rules
        .iter()
        .copied()
        .filter(|r| enabled.contains(r))
        .chain(inherited)
        .min();
    if nodes[node].children == [None, None] {
        return Partition::Leaf(winner);
    }
    let left = partition(
        nodes,
        nodes[node].children[0].map(|i| i as usize),
        winner,
        enabled,
    );
    let right = partition(
        nodes,
        nodes[node].children[1].map(|i| i as usize),
        winner,
        enabled,
    );
    if let (Partition::Leaf(a), Partition::Leaf(b)) = (&left, &right)
        && a == b
    {
        Partition::Leaf(*a)
    } else {
        Partition::Branch(Box::new(left), Box::new(right))
    }
}
fn emit(
    partition: &Partition,
    bits: u128,
    depth: u8,
    width: u8,
    out: &mut Vec<FlattenedPrefix>,
) -> Result<(), &'static str> {
    match partition {
        Partition::Leaf(None) => {}
        Partition::Leaf(Some(rule)) => {
            if out.len() == MAX_INSTALLED_PREFIXES {
                return Err("Flattened prefix budget exceeds 16384; no partial output returned");
            }
            let network = if width == 32 {
                ipnet::Ipv4Net::new(std::net::Ipv4Addr::from(bits as u32), depth)
                    .expect("prefix depth")
                    .to_string()
            } else {
                ipnet::Ipv6Net::new(std::net::Ipv6Addr::from(bits), depth)
                    .expect("prefix depth")
                    .to_string()
            };
            out.push(FlattenedPrefix {
                network,
                rule: *rule,
            });
        }
        Partition::Branch(left, right) => {
            emit(left, bits, depth + 1, width, out)?;
            emit(
                right,
                bits | (1u128 << (width - depth - 1)),
                depth + 1,
                width,
                out,
            )?;
        }
    }
    Ok(())
}
impl Default for DestinationIndex {
    fn default() -> Self {
        Self {
            ipv4: vec![PrefixNode::default()],
            ipv6: vec![PrefixNode::default()],
            domains: vec![DomainNode::default()],
        }
    }
}
impl DestinationIndex {
    pub(crate) fn flatten(
        &self,
        enabled: &BTreeSet<u16>,
    ) -> Result<Vec<FlattenedPrefix>, &'static str> {
        let mut out = Vec::new();
        emit(
            &partition(&self.ipv4, Some(0), None, enabled),
            0,
            0,
            32,
            &mut out,
        )?;
        emit(
            &partition(&self.ipv6, Some(0), None, enabled),
            0,
            0,
            128,
            &mut out,
        )?;
        Ok(out)
    }
    pub(crate) fn covered(&self, d: &ExitMatch, enabled: &BTreeSet<u16>) -> bool {
        match d {
            ExitMatch::RuleSet(_) => unreachable!("expanded"),
            ExitMatch::DomainSuffix(s) => {
                let mut node = 0;
                for label in s.split('.').rev() {
                    let Some(next) = self.domains[node].children.get(label) else {
                        return false;
                    };
                    node = *next as usize;
                    if self.domains[node].rules.iter().any(|r| enabled.contains(r)) {
                        return true;
                    }
                }
                false
            }
            ExitMatch::Cidr(s) => {
                let net = s.parse::<IpNet>().expect("validated");
                let (nodes, bits, width) = match net.network() {
                    IpAddr::V4(ip) => (&self.ipv4, u128::from(u32::from(ip)), 32),
                    IpAddr::V6(ip) => (&self.ipv6, u128::from(ip), 128),
                };
                let mut node = 0;
                for depth in 0..=usize::from(net.prefix_len()) {
                    if nodes[node].rules.iter().any(|r| enabled.contains(r)) {
                        return true;
                    }
                    if depth == usize::from(net.prefix_len()) {
                        return fully_covered(&partition(nodes, Some(node), None, enabled));
                    }
                    let bit = ((bits >> (width - depth - 1)) & 1) as usize;
                    let Some(next) = nodes[node].children[bit] else {
                        return false;
                    };
                    node = next as usize;
                }
                false
            }
        }
    }
    pub(crate) fn insert(&mut self, d: &ExitMatch, rule: u16) {
        match d {
            ExitMatch::RuleSet(_) => unreachable!("rule sets expanded before indexing"),
            ExitMatch::Cidr(s) => {
                let net = s.parse::<IpNet>().expect("validated CIDR");
                let (nodes, bits, width) = match net.network() {
                    IpAddr::V4(ip) => (&mut self.ipv4, u128::from(u32::from(ip)), 32),
                    IpAddr::V6(ip) => (&mut self.ipv6, u128::from(ip), 128),
                };
                let mut node = 0;
                for i in 0..usize::from(net.prefix_len()) {
                    let bit = ((bits >> (width - i - 1)) & 1) as usize;
                    node = match nodes[node].children[bit] {
                        Some(next) => next as usize,
                        None => {
                            let next = nodes.len();
                            nodes.push(PrefixNode::default());
                            nodes[node].children[bit] = Some(next as u32);
                            next
                        }
                    };
                }
                if !nodes[node].rules.contains(&rule) {
                    nodes[node].rules.push(rule);
                    nodes[node].rules.sort_unstable();
                }
            }
            ExitMatch::DomainSuffix(s) => {
                let mut node = 0;
                for label in s.split('.').rev() {
                    node = match self.domains[node].children.get(label) {
                        Some(next) => *next as usize,
                        None => {
                            let next = self.domains.len();
                            self.domains.push(DomainNode::default());
                            self.domains[node]
                                .children
                                .insert(label.into(), next as u32);
                            next
                        }
                    };
                }
                if !self.domains[node].rules.contains(&rule) {
                    self.domains[node].rules.push(rule);
                    self.domains[node].rules.sort_unstable();
                }
            }
        }
    }
    /// Return candidate rule IDs. Precedence is ordered first-match across both
    /// domain and IP matches; trie depth and match kind confer no priority.
    pub(crate) fn matching(&self, domain: Option<&str>, ip: Option<IpAddr>) -> Vec<u16> {
        let mut matches = Vec::new();
        if let Some(ip) = ip {
            let (nodes, bits, width) = match ip {
                IpAddr::V4(ip) => (&self.ipv4, u128::from(u32::from(ip)), 32),
                IpAddr::V6(ip) => (&self.ipv6, u128::from(ip), 128),
            };
            let mut node = 0;
            for depth in 0..=width {
                matches.extend(nodes[node].rules.iter().copied());
                if depth == width {
                    break;
                }
                let bit = ((bits >> (width - depth - 1)) & 1) as usize;
                let Some(next) = nodes[node].children[bit] else {
                    break;
                };
                node = next as usize;
            }
        }
        if let Some(domain) = domain {
            let mut node = 0;
            for label in domain.split('.').rev() {
                let Some(next) = self.domains[node].children.get(label) else {
                    break;
                };
                node = *next as usize;
                matches.extend(self.domains[node].rules.iter().copied());
            }
        }
        matches
    }
}
pub(crate) fn external_selector(s: &str) -> Option<ExitMatch> {
    s.strip_prefix("ip:")
        .map(|s| ExitMatch::Cidr(s.into()))
        .or_else(|| {
            s.strip_prefix("domain:")
                .map(|s| ExitMatch::DomainSuffix(s.into()))
        })
}
