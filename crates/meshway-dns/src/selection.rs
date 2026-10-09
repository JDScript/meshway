use crate::Query;
use hickory_proto::{
    op::Message,
    rr::{Name, RData, Record, RecordType, rdata},
};
use meshway_configuration::{
    DnsServer, NodeAddresses, execution::DnsExecution, normalize_dns_name,
};
use std::{collections::BTreeMap, fmt, net::IpAddr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigurationError {
    #[error("invalid DNS name in execution configuration")]
    Name,
    #[error("duplicate normalized DNS name in execution configuration")]
    DuplicateName,
    #[error("published host is outside the MagicDNS domain")]
    HostOutsideDomain,
    #[error("missing or unknown configured resolver")]
    ResolverReference,
    #[error("invalid configured resolver")]
    Resolver,
}

/// Caller-supplied selection fact, not execution authority or a route grant.
/// It must not be inferred from a profile name or the local node's exit role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitSelection {
    NoExit,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionUnavailable {
    /// This publication is incomplete; absence is not authoritative NXDOMAIN.
    DiscoveryRequired,
    /// Existing DnsExecution carries no executable exit DNS plan.
    MissingExitPlan,
    /// Resolver configuration exists, but no approved shared route was supplied.
    MissingResolverPlan,
    /// The local projection only implements A and AAAA answers.
    UnsupportedRecordType,
}

/// Configuration information only. Every candidate, including useWithExit,
/// requires independent upstream route authorization and execution validity.
#[derive(Clone, Copy, Debug)]
pub struct ResolverCandidate<'a> {
    pub key: &'a str,
    pub server: &'a DnsServer,
}

#[derive(Debug)]
pub enum Selection<'a> {
    Published(PublishedAnswer<'a>),
    Resolvers(Vec<ResolverCandidate<'a>>),
    /// Published single-exit service; the owner still validates its live permit.
    ExitResolver,
    /// Preserve normal host resolution; this is not a bootstrap socket grant.
    Unmanaged,
    Unavailable(SelectionUnavailable),
}

/// A published A/AAAA value or a known host with a null requested address family.
/// It contains no publication/lease validity claim.
#[derive(Debug)]
pub struct PublishedAnswer<'a> {
    query: &'a Query,
    address: Option<IpAddr>,
}

impl PublishedAnswer<'_> {
    /// Build NOERROR, with no answers for a known null address family (NODATA).
    /// `ttl_seconds` is a serialization parameter, never execution authority.
    /// The caller must bound it by the actual publication/record lifetime.
    /// AA/RA/AD remain clear; an incomplete host projection supplies no SOA.
    pub fn response(&self, ttl_seconds: u32) -> Message {
        let mut response = self.query.response();
        if let Some(address) = self.address {
            let data = match address {
                IpAddr::V4(address) => RData::A(rdata::A(address)),
                IpAddr::V6(address) => RData::AAAA(rdata::AAAA(address)),
            };
            response.add_answer(Record::from_rdata(
                self.query.question().name().clone(),
                ttl_seconds,
                data,
            ));
        }
        response
    }
}

/// Immutable projection of one DnsExecution's records and resolver settings.
/// Construction checks selection structure, not authorization or publication
/// freshness. Associations and profile strings are deliberately not inputs to
/// selection. Replace the snapshot under the eventual DNS owner's lifecycle.
#[derive(Clone)]
pub struct PublishedDns {
    magic_dns: bool,
    domain: Name,
    hosts: BTreeMap<Name, NodeAddresses>,
    servers: BTreeMap<String, DnsServer>,
    routes: BTreeMap<Name, Vec<String>>,
    default: Vec<String>,
    override_local: bool,
    executable_exit: bool,
    complete: bool,
}

impl fmt::Debug for PublishedDns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublishedDns").finish_non_exhaustive()
    }
}

impl PublishedDns {
    pub fn from_execution(execution: &DnsExecution) -> Result<Self, ConfigurationError> {
        Self::from_parts(
            &execution.settings,
            &execution.mesh_hosts,
            matches!(
                execution.egress,
                meshway_configuration::execution::DnsEgress::ViaExit { .. }
            ),
            false,
        )
    }

    /// A netmap's names are the complete set this Node may discover: itself
    /// and every Node it has Access to. Absence is then authoritative locally.
    pub fn from_netmap(
        netmap: &meshway_configuration::netmap::NodeNetmap,
    ) -> Result<Self, ConfigurationError> {
        Self::from_parts(
            &netmap.projection.configuration.dns,
            &netmap.dns.mesh_hosts,
            matches!(
                netmap.dns.egress,
                meshway_configuration::netmap::DnsPlan::ViaExit { .. }
            ),
            true,
        )
    }

    /// Whether `select`'s `DiscoveryRequired` means "not visible".
    pub fn complete(&self) -> bool {
        self.complete
    }

    fn from_parts(
        settings: &meshway_configuration::DnsConfiguration,
        mesh_hosts: &BTreeMap<String, NodeAddresses>,
        executable_exit: bool,
        complete: bool,
    ) -> Result<Self, ConfigurationError> {
        let domain = configuration_name(&settings.domain)?;
        let mut hosts = BTreeMap::new();
        for (name, addresses) in mesh_hosts {
            let name = configuration_name(name)?;
            if !domain.zone_of(&name) || domain == name {
                return Err(ConfigurationError::HostOutsideDomain);
            }
            if hosts.insert(name, addresses.clone()).is_some() {
                return Err(ConfigurationError::DuplicateName);
            }
        }
        for (key, server) in &settings.servers {
            if key.is_empty() || server.server.is_unspecified() || server.server.is_multicast() {
                return Err(ConfigurationError::Resolver);
            }
        }
        let check_references = |keys: &[String]| {
            if keys.iter().any(|key| !settings.servers.contains_key(key)) {
                Err(ConfigurationError::ResolverReference)
            } else {
                Ok(())
            }
        };
        check_references(&settings.default)?;
        if settings.override_local && settings.default.is_empty() {
            return Err(ConfigurationError::ResolverReference);
        }
        let mut routes = BTreeMap::new();
        for (suffix, keys) in &settings.routes {
            if keys.is_empty() {
                return Err(ConfigurationError::ResolverReference);
            }
            check_references(keys)?;
            if routes
                .insert(configuration_name(suffix)?, keys.clone())
                .is_some()
            {
                return Err(ConfigurationError::DuplicateName);
            }
        }
        Ok(Self {
            magic_dns: settings.magic_dns,
            domain,
            hosts,
            servers: settings.servers.clone(),
            routes,
            default: settings.default.clone(),
            override_local: settings.override_local,
            executable_exit,
            complete,
        })
    }

    /// MagicDNS precedes split selection. Retained split routes use the longest
    /// label suffix. In exit mode, only useWithExit references remain eligible;
    /// empty routes drop out before longest-suffix selection. A retained default
    /// is usable even with overrideLocal=false, which only controls no-exit mode.
    /// Otherwise exit resolution is explicitly unavailable until a shared exit
    /// DNS plan exists. Candidate order follows each configured key list.
    pub fn select<'a>(&'a self, query: &'a Query, exit: ExitSelection) -> Selection<'a> {
        let name = query.question().name();
        if self.magic_dns && self.domain.zone_of(name) {
            let Some(addresses) = self.hosts.get(name) else {
                return Selection::Unavailable(SelectionUnavailable::DiscoveryRequired);
            };
            let address = match query.question().query_type() {
                RecordType::A => addresses.ipv4.map(IpAddr::V4),
                RecordType::AAAA => addresses.ipv6.map(IpAddr::V6),
                _ => {
                    return Selection::Unavailable(SelectionUnavailable::UnsupportedRecordType);
                }
            };
            return Selection::Published(PublishedAnswer { query, address });
        }
        let eligible =
            |key: &String| exit == ExitSelection::NoExit || self.servers[key].use_with_exit;
        let route = self
            .routes
            .iter()
            .filter(|(suffix, keys)| suffix.zone_of(name) && keys.iter().any(eligible))
            .max_by_key(|(suffix, _)| suffix.num_labels());
        let keys = if let Some((_, keys)) = route {
            keys
        } else if exit == ExitSelection::Exit || self.override_local {
            &self.default
        } else {
            return Selection::Unmanaged;
        };
        let candidates: Vec<_> = keys
            .iter()
            .filter(|key| eligible(key))
            .map(|key| ResolverCandidate {
                key,
                server: &self.servers[key],
            })
            .collect();
        if candidates.is_empty() {
            if exit == ExitSelection::Exit && self.executable_exit {
                Selection::ExitResolver
            } else {
                Selection::Unavailable(SelectionUnavailable::MissingExitPlan)
            }
        } else {
            Selection::Resolvers(candidates)
        }
    }
}

fn configuration_name(value: &str) -> Result<Name, ConfigurationError> {
    // Reuse the configuration's bounded DNS normalization for settings;
    // DNS wire names themselves retain arbitrary labels and use Name::zone_of.
    let normalized = normalize_dns_name(value).map_err(|_| ConfigurationError::Name)?;
    let mut name = Name::from_ascii(normalized).map_err(|_| ConfigurationError::Name)?;
    name.set_fqdn(true);
    Ok(name)
}

#[cfg(test)]
mod tests;
