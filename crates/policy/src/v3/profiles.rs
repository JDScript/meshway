//! Product-level routing schemes, built on one network-global three-layer plan.
use super::*;
pub const MESH_ONLY: &str = "mesh-only";
pub const AUTO: &str = "auto";

/// Concrete profile identity shared by compiler and execution adapters. Custom
/// IDs retain the existing 1..64 ASCII syntax; global IDs retain all 64 hex digits.
/// `global:*` is an assignment expansion token, never a concrete profile ID.
pub fn valid_profile_id(id: &str) -> bool {
    id.strip_prefix("global:")
        .map_or_else(|| valid_custom_profile_id(id), super::validation::valid_id)
}
pub(super) fn valid_custom_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    pub connections: Vec<ConnectionRule>,
    pub grants: Vec<Grant>,
    pub mesh: Vec<MeshRule>,
    pub public_rules: Vec<ExitRule>,
    pub profiles: Vec<RoutingProfile>,
    #[serde(default)]
    pub automatic: AutomaticRouting,
    pub assignments: Vec<ProfileAssignment>,
}
impl NetworkPolicy {
    pub fn default_template() -> Self {
        let p = Policy::default_template();
        Self {
            connections: p.connections,
            grants: p.grants,
            mesh: p.mesh,
            ..Self::default()
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutingProfile {
    pub id: String,
    pub rules: Vec<ExitRule>,
    pub fallback: ProfileFallback,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProfileFallback {
    // Empty struct variants are intentional: serde's internally tagged unit
    // variants ignore extra fields even with deny_unknown_fields on the enum.
    // utoipa does not propagate the container's deny_unknown_fields to variant
    // schemas. All keys are required, so max_properties closes each variant.
    #[schema(max_properties = 1)]
    None {},
    #[schema(max_properties = 1)]
    Direct {},
    #[schema(max_properties = 3)]
    Nodes {
        candidates: Vec<String>,
        via: Vec<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AutomaticRouting {
    pub regions: BTreeMap<String, String>,
    pub fallback: String,
}
impl Default for AutomaticRouting {
    fn default() -> Self {
        Self {
            regions: BTreeMap::new(),
            fallback: MESH_ONLY.into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileAssignment {
    pub src: String,
    pub allowed: Vec<String>,
    pub default_profile: String,
    pub locked: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileMatcher {
    pub rules: Vec<ExitRule>,
    pub index: DestinationIndex,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileRoute {
    pub rule: u16,
    pub target: ExitTarget,
    pub paths: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(
    tag = "kind",
    content = "paths",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum CompiledFinal {
    None,
    Direct,
    Nodes(Vec<u32>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledProfile {
    pub id: String,
    pub matcher: u16,
    pub routes: Vec<ProfileRoute>,
    pub ip_table: u32,
    pub fallback: CompiledFinal,
    pub lints: Vec<ExitLint>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceProfiles {
    pub allowed: Vec<String>,
    pub default_profile: String,
    pub locked: bool,
    pub profiles: Vec<CompiledProfile>,
    pub automatic: AutomaticRouting,
}
/// Trusted in-process output; serialization is diagnostic only. Reconstruct via
/// `compile_network`, never by decoding compiled indexes or authorizations.
///
/// ```compile_fail
/// let _: meshway_policy::v3::NetworkCompilation = serde_json::from_str("{}").unwrap();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkCompilation {
    pub base: Compiled,
    pub matchers: Vec<ProfileMatcher>,
    pub ip_tables: Vec<Vec<FlattenedPrefix>>,
    pub sources: Vec<SourceProfiles>,
}
/// Only the caller's STUN-observed public-IP geolocation, never controller peer IP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StunLocation {
    pub region: Option<String>,
}
impl NetworkCompilation {
    pub fn select_profile(
        &self,
        src: usize,
        requested: Option<&str>,
        location: &StunLocation,
    ) -> Result<&CompiledProfile, &'static str> {
        let source = self.sources.get(src).ok_or("Unknown source")?;
        let selected = if source.locked {
            source.default_profile.as_str()
        } else {
            requested.unwrap_or(&source.default_profile)
        };
        if !source.allowed.iter().any(|p| p == selected) {
            return Err("Profile not allowed for source");
        }
        let selected = if selected == AUTO {
            location
                .region
                .as_ref()
                .and_then(|s| source.automatic.regions.get(&s.to_ascii_uppercase()))
                .unwrap_or(&source.automatic.fallback)
                .as_str()
        } else {
            selected
        };
        source
            .profiles
            .iter()
            .find(|p| p.id == selected)
            .ok_or("Automatic target not allowed for source")
    }
    pub fn exit_for_profile(
        &self,
        src: usize,
        requested: Option<&str>,
        location: &StunLocation,
        domain: Option<&str>,
        ip: Option<std::net::IpAddr>,
    ) -> Result<ExitDecision<'_>, &'static str> {
        let profile = self.select_profile(src, requested, location)?;
        let domain = domain
            .map(meshway_protocol::normalize_dns_name)
            .transpose()?;
        if matches!(ip,Some(std::net::IpAddr::V6(v)) if v.to_ipv4_mapped().is_some()) {
            return Err("Mapped IPv6 unsupported");
        }
        let matcher = &self.matchers[usize::from(profile.matcher)];
        let rule = matcher
            .index
            .matching(domain.as_deref(), ip)
            .into_iter()
            .filter_map(|rule| {
                profile
                    .routes
                    .binary_search_by_key(&rule, |r| r.rule)
                    .ok()
                    .map(|i| &profile.routes[i])
            })
            .min_by_key(|r| r.rule);
        let authorized = self.base.external_authorized(src, domain.as_deref(), ip);
        match rule {
            Some(r) if r.target == ExitTarget::Direct => Ok(ExitDecision::Local),
            Some(_) if !authorized => Ok(ExitDecision::Denied),
            Some(r) if r.paths.is_empty() => Ok(ExitDecision::Unreachable),
            Some(r) => Ok(ExitDecision::Paths(&r.paths)),
            None => match &profile.fallback {
                CompiledFinal::None | CompiledFinal::Direct => Ok(ExitDecision::Local),
                CompiledFinal::Nodes(_) if ip.is_none() => {
                    Err("Resolved IP required for profile final")
                }
                CompiledFinal::Nodes(_) if !authorized => Ok(ExitDecision::Denied),
                CompiledFinal::Nodes(paths) if paths.is_empty() => Ok(ExitDecision::Unreachable),
                CompiledFinal::Nodes(paths) => Ok(ExitDecision::Paths(paths)),
            },
        }
    }
}

/// Product entry point. Static rejection precedes the all-pairs kernel compile;
/// authorization coverage still uses that one shared, compiled three-layer plan.
pub fn compile_network(
    p: &NetworkPolicy,
    facts: &Snapshot,
    options: CompileOptions,
) -> Result<NetworkCompilation, Vec<Diagnostic>> {
    super::profile_validation::check_budget(p)?;
    let intent = Policy {
        connections: p.connections.clone(),
        grants: p.grants.clone(),
        mesh: p.mesh.clone(),
        ..Policy::default()
    };
    // Keep kernel input diagnostics ahead of profile diagnostics, without doing
    // any graph construction or all-pairs path search on invalid input.
    let expanded = expand_rule_sets(&intent, &facts.rule_sets)?;
    let mut diagnostics = super::validation::validate_expanded(&expanded, facts);
    if !(1..=2).contains(&options.paths) {
        diagnostics.push(Diagnostic {
            field: "options.paths".into(),
            message: "Require 1 or 2 paths".into(),
        });
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let catalog = super::profile_validation::preflight(p, facts)?;
    let matchers = super::profile_plan::matchers(&catalog)?;
    let mut base = compile(&intent, facts, options)?;
    let mut plan = super::profile_plan::plan(p, facts, &base, &catalog, &matchers)?;
    let path_ids = super::engine::append_external_requests(
        &intent,
        facts,
        options,
        &mut base,
        std::mem::take(&mut plan.requests),
    )?;
    Ok(super::profile_projection::project(
        p, base, matchers, plan, path_ids,
    ))
}
#[cfg(test)]
#[path = "profiles_tests.rs"]
mod tests;
