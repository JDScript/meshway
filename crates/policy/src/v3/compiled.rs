//! Trusted kernel compilation output, lookup and directional projections.
use super::*;
use std::net::IpAddr;

#[derive(Clone, Copy, Debug)]
pub struct CompileOptions {
    /// One primary plus at most one logical backup.
    pub paths: usize,
}
impl Default for CompileOptions {
    fn default() -> Self {
        Self { paths: 2 }
    }
}
/// Node indexes refer to the sorted `nodes` table; path indexes to `paths`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Compiled {
    pub nodes: Vec<String>,
    pub paths: Vec<Vec<u16>>,
    pub path_scopes: Vec<PathScope>,
    pub mesh: Vec<MeshFlow>,
    pub connections: BTreeSet<(u16, u16)>,
    pub per_node: Vec<Vec<Forwarding>>,
    pub exits: Vec<Vec<ExitEntry>>,
    pub defaults: Vec<Vec<DefaultEntry>>,
    pub exit_rules: Vec<ExitRule>,
    pub exit_index: DestinationIndex,
    pub external_grants: Vec<ExternalAuthorization>,
    pub grant_index: DestinationIndex,
    pub ip_tables: Vec<Vec<super::index::FlattenedPrefix>>,
    pub source_ip_table: Vec<u16>,
    pub lints: Vec<ExitLint>,
    pub failures: Vec<FlowFailure>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    Mesh,
    External,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshFlow {
    pub src: u16,
    pub dst: u16,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Forwarding {
    pub path: u32,
    pub previous: Option<u16>,
    pub next: Option<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitEntry {
    pub rule_id: String,
    pub rule: u16,
    pub target: ExitTarget,
    pub specificity: u8,
    pub authorized: bool,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultCandidate {
    pub node: u16,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DefaultEntry {
    pub family: AddressFamily,
    pub locked: bool,
    pub authorized: bool,
    pub target: ExitTarget,
    pub candidates: Vec<DefaultCandidate>,
    /// Ordered primary/standby paths used when locked; selection uses one candidate.
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalAuthorization {
    pub internet: bool,
    pub range_grants: BTreeSet<u16>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitLint {
    pub rule_id: String,
    pub shadowed_destinations: usize,
    pub total_destinations: usize,
    pub fully_shadowed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowFailure {
    pub src: u16,
    pub destination: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExitDecision<'a> {
    Local,
    Denied,
    Unreachable,
    Paths(&'a [u32]),
}
impl Compiled {
    /// Caller supplies trusted domain context, never inferred from an arbitrary IP packet.
    pub fn exit_for(
        &self,
        src: usize,
        domain: Option<&str>,
        ip: Option<IpAddr>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        self.exit_for_selected(src, domain, ip, None)
    }
    pub fn exit_for_selected(
        &self,
        src: usize,
        domain: Option<&str>,
        ip: Option<IpAddr>,
        selection: Option<u16>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        let domain = domain
            .map(meshway_protocol::normalize_dns_name)
            .transpose()?;
        if matches!(ip,Some(IpAddr::V6(v)) if v.to_ipv4_mapped().is_some()) {
            return Err("Mapped IPv6 unsupported");
        }
        let entries = self.exits.get(src).ok_or("Unknown source index")?;
        let grants = &self.external_grants[src];
        let authorized = grants.internet
            || self
                .grant_index
                .matching(domain.as_deref(), ip)
                .iter()
                .any(|g| grants.range_grants.contains(g));
        let best = self
            .exit_index
            .matching(domain.as_deref(), ip)
            .into_iter()
            .filter_map(|rule| {
                let i = entries.binary_search_by_key(&rule, |e| e.rule).ok()?;
                Some(&entries[i])
            })
            .min_by_key(|e| e.rule);
        Ok(match best {
            None => {
                let family = match ip {
                    Some(IpAddr::V4(_)) => Some(AddressFamily::Ipv4),
                    Some(IpAddr::V6(_)) => Some(AddressFamily::Ipv6),
                    None => None,
                };
                let defaults = &self.defaults[src];
                let entry = family.and_then(|family| defaults.iter().find(|e| e.family == family));
                if family.is_none() && (selection.is_some() || defaults.iter().any(|e| e.locked)) {
                    return Err("Resolved IP family required for default exit");
                }
                match entry {
                    None if selection.is_some() => ExitDecision::Denied,
                    None => ExitDecision::Local,
                    Some(e) if !e.locked && selection.is_none() => ExitDecision::Local,
                    Some(e) if e.target == ExitTarget::Direct => ExitDecision::Local,
                    Some(_) if !authorized => ExitDecision::Denied,
                    Some(e) if e.locked => {
                        if e.paths.is_empty() {
                            ExitDecision::Unreachable
                        } else {
                            ExitDecision::Paths(&e.paths)
                        }
                    }
                    Some(e) => match e.candidates.iter().find(|c| Some(c.node) == selection) {
                        None => ExitDecision::Denied,
                        Some(c) if c.paths.is_empty() => ExitDecision::Unreachable,
                        Some(c) => ExitDecision::Paths(&c.paths),
                    },
                }
            }
            Some(e) if e.target == ExitTarget::Direct => ExitDecision::Local,
            Some(_) if !authorized => ExitDecision::Denied,
            Some(e) if e.paths.is_empty() => ExitDecision::Unreachable,
            Some(e) => ExitDecision::Paths(&e.paths),
        })
    }
    /// Same complete, directional path semantics as MeshPathAuthorization. Legacy
    /// publication budgets and v2 routing intersection still require integration.
    pub fn exact_authorization(&self, node: usize) -> meshway_protocol::MeshPathAuthorization {
        self.exact_paths(node, PathScope::Mesh)
    }
    /// This projection must be installed together with external destination grants
    /// and route context. It does not authorize Mesh access to the exit node itself.
    pub fn external_path_authorization(
        &self,
        node: usize,
    ) -> meshway_protocol::MeshPathAuthorization {
        self.exact_paths(node, PathScope::External)
    }
    fn exact_paths(
        &self,
        node: usize,
        scope: PathScope,
    ) -> meshway_protocol::MeshPathAuthorization {
        let mut unique = BTreeSet::new();
        meshway_protocol::MeshPathAuthorization {
            grants: self.per_node[node]
                .iter()
                .filter(|f| self.path_scopes[f.path as usize] == scope)
                .filter(|f| self.paths[f.path as usize].len() > 1)
                .filter(|f| unique.insert(&self.paths[f.path as usize]))
                .map(|f| meshway_protocol::MeshAccessGrant {
                    id: format!("v3-{}", f.path),
                    path: self.paths[f.path as usize]
                        .iter()
                        .map(|i| self.nodes[usize::from(*i)].clone())
                        .collect(),
                })
                .collect(),
        }
    }
}
