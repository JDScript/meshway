//! Single-network identity and authoritative node DNS intent, not recursive DNS
//! or access authorization. AAAA metadata does not assert IPv6 dataplane support.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use idna::uts46::{AsciiDenyList, DnsLength, Hyphens, Uts46};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use crate::{MeshNetwork, MeshNode, PolicyDiagnostic, canonical_json};

pub const DNS_SCHEMA_VERSION: u32 = 1;
pub const MAX_DNS_BYTES: usize = 1024 * 1024;
pub const MAX_DNS_RECORDS: usize = 1024;
pub const MAX_DNS_DIAGNOSTICS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeshDnsConfiguration {
    pub enabled: bool,
    #[schema(min_length = 3, max_length = 128)]
    pub zone: String,
    #[schema(minimum = 1, maximum = 300)]
    pub ttl_seconds: u32,
}

impl Default for MeshDnsConfiguration {
    fn default() -> Self {
        Self {
            enabled: false,
            zone: "meshway.test".into(),
            ttl_seconds: 30,
        }
    }
}

impl MeshDnsConfiguration {
    pub fn normalized(&self) -> Result<Self, &'static str> {
        if !(1..=300).contains(&self.ttl_seconds) {
            return Err("DNS TTL must be between 1 and 300 seconds");
        }
        let zone = normalize_dns_name(&self.zone)?;
        if zone.len() > 128 || !zone.contains('.') || reserved_zone(&zone) {
            return Err(
                "DNS zone must have at least two labels, at most 128 ASCII bytes, and not be a reserved zone",
            );
        }
        Ok(Self {
            zone,
            ..self.clone()
        })
    }
}

/// Saved intent at the global configuration revision. Address pools are immutable.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkSnapshot {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 32, max_length = 32, pattern = "^[0-9a-f]{32}$")]
    pub network_id: String,
    #[schema(min_length = 1, max_length = 128)]
    pub name: String,
    pub network: MeshNetwork,
    pub dns: MeshDnsConfiguration,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveNetworkRequest {
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub expected_revision: u32,
    #[schema(min_length = 1, max_length = 128)]
    pub name: String,
    pub dns: MeshDnsConfiguration,
}

impl SaveNetworkRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        crate::validate_text(&self.name, 128)?;
        self.dns.normalized()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeDnsRecord {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub node_id: String,
    #[schema(min_length = 1, max_length = 253)]
    pub canonical_name: String,
    #[schema(max_items = 1)]
    pub aliases: Vec<String>,
    pub mesh_ipv4: String,
    pub mesh_ipv6: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeDnsConfiguration {
    #[schema(minimum = 1, maximum = 1)]
    pub schema_version: u32,
    #[schema(format = Int64, minimum = 1, maximum = 4294967295_u64)]
    pub revision: u32,
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub node_id: String,
    #[schema(min_length = 32, max_length = 32, pattern = "^[0-9a-f]{32}$")]
    pub network_id: String,
    pub dns: MeshDnsConfiguration,
    #[schema(max_items = 1024)]
    pub records: Vec<NodeDnsRecord>,
    #[schema(max_items = 64)]
    pub diagnostics: Vec<PolicyDiagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentDnsSnapshot {
    #[schema(min_length = 64, max_length = 64, pattern = "^[0-9a-f]{64}$")]
    pub digest: String,
    pub payload: NodeDnsConfiguration,
}

pub fn normalize_dns_name(value: &str) -> Result<String, &'static str> {
    let ascii = Uts46::new()
        .to_ascii(
            value.as_bytes(),
            AsciiDenyList::STD3,
            Hyphens::Check,
            DnsLength::VerifyAllowRootDot,
        )
        .map_err(|_| "DNS name must be valid IDNA without wildcards or URL syntax")?;
    let name = ascii
        .strip_suffix('.')
        .unwrap_or(&ascii)
        .to_ascii_lowercase();
    if name.is_empty() || name.parse::<IpAddr>().is_ok() {
        return Err("DNS name must be nonempty and not an IP address");
    }
    Ok(name)
}

fn reserved_zone(zone: &str) -> bool {
    ["localhost", "in-addr.arpa", "ip6.arpa"]
        .iter()
        .any(|reserved| zone == *reserved || zone.ends_with(&format!(".{reserved}")))
}

/// The full durable node identity is split into legal DNS labels, never truncated.
pub fn canonical_node_dns_name(node_id: &str, zone: &str) -> Result<String, &'static str> {
    if !crate::local::valid_credential(node_id) {
        return Err("DNS record requires a 64-character lowercase hexadecimal node ID");
    }
    Ok(format!("id.{}.{}.{zone}", &node_id[..32], &node_id[32..]))
}

fn alias_label(name: &str) -> Option<String> {
    let label = normalize_dns_name(name).ok()?;
    (!label.contains('.') && label != "id" && !reserved_zone(&label)).then_some(label)
}

/// Input is the entire active public inventory, including nodes without public
/// keys. Invalid or ambiguous display names never replace canonical identities.
pub fn compile_dns_records(
    dns: &MeshDnsConfiguration,
    nodes: &[MeshNode],
) -> Result<(Vec<NodeDnsRecord>, Vec<PolicyDiagnostic>), &'static str> {
    if nodes.len() > MAX_DNS_RECORDS {
        return Err("DNS inventory exceeds 1024 records");
    }
    if dns.normalized()? != *dns {
        return Err("DNS configuration must be normalized");
    }
    if !dns.enabled {
        return Ok((vec![], vec![]));
    }
    let mut counts = BTreeMap::<String, usize>::new();
    for node in nodes {
        if let Some(label) = alias_label(&node.name) {
            *counts.entry(label).or_default() += 1;
        }
    }
    let mut sorted: Vec<_> = nodes.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut records = Vec::new();
    let mut diagnostics = Vec::new();
    let mut previous = None;
    for node in sorted {
        if previous == Some(&node.id) {
            return Err("duplicate DNS node identity");
        }
        previous = Some(&node.id);
        let canonical_name = canonical_node_dns_name(&node.id, &dns.zone)?;
        if node.mesh_ipv4.parse::<Ipv4Addr>().is_err()
            || node.mesh_ipv6.parse::<Ipv6Addr>().is_err()
        {
            return Err("invalid DNS assigned address");
        }
        let label = alias_label(&node.name);
        let aliases = match label {
            Some(label) if counts.get(&label) == Some(&1) => vec![format!("{label}.{}", dns.zone)],
            _ => {
                if diagnostics.len() < MAX_DNS_DIAGNOSTICS {
                    diagnostics.push(PolicyDiagnostic { rule_id: None, field: "records.aliases".into(),
                        message: "Display name is not a unique valid DNS label; canonical identity name remains available".into() });
                }
                vec![]
            }
        };
        records.push(NodeDnsRecord {
            node_id: node.id.clone(),
            canonical_name,
            aliases,
            mesh_ipv4: node.mesh_ipv4.clone(),
            mesh_ipv6: node.mesh_ipv6.clone(),
        });
    }
    Ok((records, diagnostics))
}

impl NodeDnsConfiguration {
    /// Validate authenticated wire/cache data before publication. Names must be
    /// strictly within this zone and agree with metadata identity.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != DNS_SCHEMA_VERSION
            || self.revision == 0
            || !crate::local::valid_credential(&self.node_id)
            || self.network_id.len() != 32
            || !self
                .network_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid DNS snapshot metadata");
        }
        if self.dns.normalized()? != self.dns {
            return Err("DNS configuration must be normalized");
        }
        if self.records.len() > MAX_DNS_RECORDS
            || self.diagnostics.len() > MAX_DNS_DIAGNOSTICS
            || (!self.dns.enabled && !self.records.is_empty())
        {
            return Err("invalid DNS publication bounds");
        }
        let mut names = BTreeSet::new();
        let mut previous: Option<&str> = None;
        for record in &self.records {
            if previous.is_some_and(|id| id >= record.node_id.as_str())
                || record.canonical_name
                    != canonical_node_dns_name(&record.node_id, &self.dns.zone)?
                || record.aliases.len() > 1
                || record.mesh_ipv4.parse::<Ipv4Addr>().is_err()
                || record.mesh_ipv6.parse::<Ipv6Addr>().is_err()
            {
                return Err("invalid DNS record metadata or order");
            }
            previous = Some(&record.node_id);
            if !names.insert(record.canonical_name.clone()) {
                return Err("duplicate DNS name");
            }
            for alias in &record.aliases {
                let label = alias
                    .strip_suffix(&format!(".{}", self.dns.zone))
                    .ok_or("DNS alias outside authoritative zone")?;
                if alias_label(label).as_deref() != Some(label) || !names.insert(alias.clone()) {
                    return Err("invalid or duplicate DNS alias");
                }
            }
        }
        if canonical_json(self)
            .map_err(|_| "cannot encode DNS configuration")?
            .len()
            > MAX_DNS_BYTES
        {
            return Err("DNS configuration exceeds 1 MiB");
        }
        Ok(())
    }
}

pub fn dns_digest(payload: &NodeDnsConfiguration) -> Result<String, serde_json::Error> {
    Ok(Sha256::digest(canonical_json(payload)?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
