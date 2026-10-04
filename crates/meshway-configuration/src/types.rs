use crate::{
    fields::*,
    object::{object, tagged, unique_map},
};
use meshway_model::NodeId;
use meshway_transport_spec::{DialerSet, Listener, Port, PositiveDuration, PreparedListener};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
use utoipa::ToSchema;

object! { pub struct AddressPools { pub ipv4: Vec<String>, pub ipv6: Vec<String> } }
object! { #[derive(Default)] pub struct AddressPoolsInput {
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv4: Input<Vec<String>> [value_type = Vec<String>],
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv6: Input<Vec<String>> [value_type = Vec<String>],
} }
object! { pub struct NodeAddresses {
    #[serde(deserialize_with = "nullable")] pub ipv4: Option<Ipv4Addr> [value_type = Option<String>, required = true, format = "ipv4"],
    #[serde(deserialize_with = "nullable")] pub ipv6: Option<Ipv6Addr> [value_type = Option<String>, required = true, format = "ipv6"],
} }
fn nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
object! { #[derive(Default)] pub struct NodeAddressesInput {
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv4: Input<Option<Ipv4Addr>> [value_type = Option<String>],
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub ipv6: Input<Option<Ipv6Addr>> [value_type = Option<String>],
} }
object! { pub struct Defaults {
    #[serde(default)] pub access: AccessDefault,
    #[serde(default)] pub topology: TopologyDefault,
    #[serde(default = "DialerSet::network_default")] pub dialers: DialerSet,
} }
impl Default for Defaults {
    fn default() -> Self {
        Self {
            access: AccessDefault::Deny,
            topology: TopologyDefault::Allow,
            dialers: DialerSet::network_default(),
        }
    }
}

tagged! { pub enum NodeIdentity { User { user: String }, Tag { tags: Vec<String> } } }

object! { pub struct AccessRule {
    #[serde(default)] pub remark: String,
    pub src: Vec<String>, pub dst: Vec<String>, pub ip: Vec<String>,
    #[serde(default)] pub bidirectional: bool,
} }
object! { pub struct TopologyRule {
    #[serde(default)] pub remark: String,
    pub action: TopologyAction, pub a: String, pub b: String,
} }
fn yes() -> bool {
    true
}
object! { pub struct NodeSettings {
    pub hostname: String,
    #[serde(default)] pub remark: String,
    pub identity: NodeIdentity,
    #[serde(default = "yes")] pub enabled: bool,
    pub addresses: NodeAddresses,
    #[serde(default, deserialize_with = "unique_map")] pub listeners: BTreeMap<String, PreparedListener>,
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub dialers: Input<DialerSet> [value_type = DialerSet],
    #[serde(default)] pub exit_enabled: bool,
} }
object! { pub struct NodeSettingsInput {
    pub hostname: String,
    #[serde(default)] pub remark: String,
    pub identity: NodeIdentity,
    #[serde(default = "yes")] pub enabled: bool,
    #[serde(default)] pub addresses: NodeAddressesInput,
    #[serde(default, deserialize_with = "unique_map")] pub listeners: BTreeMap<String, Listener>,
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub dialers: Input<DialerSet> [value_type = DialerSet],
    #[serde(default)] pub exit_enabled: bool,
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum DnsTransport {
    Udp,
    Tcp,
}
impl TryFrom<String> for DnsTransport {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "udp" => Ok(Self::Udp),
            "tcp" => Ok(Self::Tcp),
            _ => Err("unsupported DNS transport"),
        }
    }
}
fn dns_port() -> Port {
    53.try_into().expect("constant port")
}
object! { pub struct DnsServer {
    #[serde(rename = "type")] pub transport: DnsTransport,
    pub server: IpAddr [value_type = String],
    #[serde(default = "dns_port")] pub server_port: Port,
    #[serde(default)] pub use_with_exit: bool,
} }
object! { pub struct DnsConfiguration {
    #[serde(default = "yes", rename = "magicDNS")] pub magic_dns: bool,
    pub domain: String,
    #[serde(default, deserialize_with = "unique_map")] pub servers: BTreeMap<String, DnsServer>,
    #[serde(default)] pub default: Vec<String>,
    #[serde(default, deserialize_with = "unique_map")] pub routes: BTreeMap<String, Vec<String>>,
    #[serde(default)] pub search_domains: Vec<String>,
    #[serde(default)] pub override_local: bool,
} }
object! { pub struct MeshRoute {
    #[serde(default)] pub remark: String,
    pub src: Vec<String>, pub dst: Vec<String>, pub via: Vec<String>,
} }
object! { #[derive(Default)] pub struct RouteMatch {
    #[serde(default)] pub domain: Vec<String>,
    #[serde(default)] pub domain_suffix: Vec<String>,
    #[serde(default, rename = "ipCIDR")] pub ip_cidr: Vec<String>,
    #[serde(default)] pub rule_sets: Vec<String>,
} }
// What a matched egress rule does with the traffic.
tagged! { pub enum RuleAction {
    Exit { nodes: Vec<String>, #[serde(default)] via: Vec<String> },
    Direct,
} }
// What a profile does with traffic no rule matched; `unmanaged` leaves it to
// the local system and is only meaningful as a fallback.
tagged! { pub enum RouteAction {
    Exit { nodes: Vec<String>, #[serde(default)] via: Vec<String> },
    Direct, Unmanaged,
} }
impl From<RuleAction> for RouteAction {
    fn from(action: RuleAction) -> Self {
        match action {
            RuleAction::Exit { nodes, via } => Self::Exit { nodes, via },
            RuleAction::Direct => Self::Direct,
        }
    }
}
object! { pub struct EgressRule {
    #[serde(default)] pub remark: String,
    #[serde(rename = "match")] pub matcher: RouteMatch,
    pub action: RuleAction,
} }
object! { pub struct RoutingProfile {
    #[serde(default)] pub rules: Vec<EgressRule>,
    pub fallback: RouteAction,
} }
object! { pub struct ProfileAssignment {
    pub target: Vec<String>, pub allowed: Vec<String>, pub default: String,
    #[serde(default)] pub locked: bool,
} }
object! { pub struct CountryProfile { pub countries: Vec<String>, pub profile: String } }
object! { pub struct AutoProfile { pub rules: Vec<CountryProfile>, pub fallback: String } }
object! { #[derive(Default)] pub struct EgressRouting {
    #[serde(default)] pub rules: Vec<EgressRule>,
    #[serde(default, deserialize_with = "unique_map")] pub profiles: BTreeMap<String, RoutingProfile>,
    #[serde(default)] pub assignments: Vec<ProfileAssignment>,
    #[serde(default, skip_serializing_if = "Input::is_omitted")] pub auto: Input<AutoProfile> [value_type = AutoProfile],
} }
object! { #[derive(Default)] pub struct RoutingConfiguration {
    #[serde(default)] pub mesh: Vec<MeshRoute>,
    #[serde(default)] pub egress: EgressRouting,
} }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "kebab-case", try_from = "String")]
pub enum RuleSetFormat {
    SingBoxSource,
    MihomoDomain,
    MihomoIpcidr,
}
impl TryFrom<String> for RuleSetFormat {
    type Error = &'static str;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        match v.as_str() {
            "sing-box-source" => Ok(Self::SingBoxSource),
            "mihomo-domain" => Ok(Self::MihomoDomain),
            "mihomo-ipcidr" => Ok(Self::MihomoIpcidr),
            _ => Err("unsupported rule set format"),
        }
    }
}
fn day() -> PositiveDuration {
    "24h".parse().expect("constant duration")
}
tagged! { pub enum RuleSet {
    Inline { #[serde(default, rename = "ipCIDR")] ip_cidr: Vec<String>, #[serde(default, rename = "domainSuffix")] domain_suffix: Vec<String>, #[serde(default)] domain: Vec<String> },
    Remote { url: String, format: RuleSetFormat, #[serde(default = "day", rename = "updateInterval")] update_interval: PositiveDuration },
} }

object! { pub struct NetworkConfiguration {
    pub address_pools: AddressPools,
    #[serde(default)] pub defaults: Defaults,
    #[serde(default, deserialize_with = "unique_map")] pub groups: BTreeMap<String, Vec<String>>,
    #[serde(default, deserialize_with = "unique_map")] pub tags_owners: BTreeMap<String, Vec<String>>,
    #[serde(default)] pub access: Vec<AccessRule>,
    #[serde(default, deserialize_with = "unique_map")] pub nodes: BTreeMap<NodeId, NodeSettings>,
    pub dns: DnsConfiguration,
    #[serde(default)] pub topology: Vec<TopologyRule>,
    #[serde(default)] pub routing: RoutingConfiguration,
    #[serde(default, deserialize_with = "unique_map")] pub rule_sets: BTreeMap<String, RuleSet>,
} }
object! { pub struct NetworkConfigurationInput {
    #[serde(default)] pub address_pools: AddressPoolsInput,
    #[serde(default)] pub defaults: Defaults,
    #[serde(default, deserialize_with = "unique_map")] pub groups: BTreeMap<String, Vec<String>>,
    #[serde(default, deserialize_with = "unique_map")] pub tags_owners: BTreeMap<String, Vec<String>>,
    #[serde(default)] pub access: Vec<AccessRule>,
    #[serde(default, deserialize_with = "unique_map")] pub nodes: BTreeMap<NodeId, NodeSettingsInput>,
    pub dns: DnsConfiguration,
    #[serde(default)] pub topology: Vec<TopologyRule>,
    #[serde(default)] pub routing: RoutingConfiguration,
    #[serde(default, deserialize_with = "unique_map")] pub rule_sets: BTreeMap<String, RuleSet>,
} }
