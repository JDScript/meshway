//! Internal P1c plan, deliberately not an HTTP contract. P3c will wire this into
//! coherent managed snapshots. Today only the isolated ignored fixture activates it.
#![allow(dead_code)]

use crate::AgentError;
use ipnet::Ipv4Net;
use meshway_dataplane::{
    LabConfig, ReceivedPacket,
    packet_io::{Admission, ipv4},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::Ipv4Addr,
    sync::Mutex,
    time::{Duration, Instant},
};

#[cfg(all(test, target_os = "linux"))]
mod fixture;
#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(test)]
mod profile_tests;

#[derive(Clone, Debug)]
pub(crate) struct ExitRoute {
    pub prefix: Ipv4Net,
    /// One selected candidate, with all mandatory intermediates.
    pub path: Vec<String>,
    pub return_path: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct ExitGrant {
    pub source: Ipv4Addr,
    pub path: Vec<String>,
    pub return_path: Vec<String>,
    pub prefixes: Vec<Ipv4Net>,
    pub authorization: ExitAuthorization,
}

#[derive(Clone, Debug)]
pub(crate) struct ExitAuthorization {
    pub internet: bool,
    pub networks: Vec<Ipv4Net>,
}

impl ExitAuthorization {
    fn allows(&self, address: Ipv4Addr) -> bool {
        let public = !address.is_private()
            && !address.is_link_local()
            && !matches!(address.octets(), [100, 64..=127, _, _]);
        (self.internet && public) || self.networks.iter().any(|prefix| prefix.contains(&address))
    }
}

/// Already-flattened public + profile rules. Devices never evaluate rule order.
#[derive(Clone, Debug)]
pub(crate) struct RoutingProfile {
    pub id: String,
    pub routes: Vec<ExitRoute>,
    pub direct_prefixes: Vec<Ipv4Net>,
    pub fallback: ProfileFallback,
}

#[derive(Clone, Debug)]
pub(crate) enum ProfileFallback {
    Exit(ExitRoute),
    Direct,
    Unmanaged,
}

/// Resolution is controller-owned and tied to a fresh Agent STUN observation;
/// it is not computed from controller connection IP or device GeoIP data.
#[derive(Clone, Debug)]
pub(crate) struct AutomaticProfile {
    pub region_to_profile: BTreeMap<String, String>,
    pub resolved_profile: Option<String>,
    pub fallback_profile: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ProfileCatalog {
    pub profiles: Vec<RoutingProfile>,
    pub default_profile: String,
    pub locked: bool,
    pub automatic: Option<AutomaticProfile>,
}

pub(crate) struct ExitPlan {
    pub local: Ipv4Addr,
    pub mesh: Ipv4Net,
    pub mtu: u16,
    pub routes: Vec<ExitRoute>,
    pub specific_routes: Vec<ExitRoute>,
    pub direct_prefixes: Vec<Ipv4Net>,
    pub profiles: Option<ProfileCatalog>,
    pub selected_profile: Option<String>,
    pub effective_profile: Option<String>,
    pub grants: Vec<ExitGrant>,
    mesh_admission: Option<super::PacketPlan>,
    flows: Mutex<BTreeMap<Flow, Instant>>,
    returns: Mutex<BTreeMap<Flow, (Vec<String>, Instant)>>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Flow {
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    source_port: u16,
    destination_port: u16,
}

impl Flow {
    fn parse(bytes: &[u8], mtu: u16) -> Option<Self> {
        let (source, destination) = ipv4(bytes, mtu)?;
        let body = bytes.get(20..)?;
        let word = |offset| {
            body.get(offset..offset + 2)
                .map(|b| u16::from_be_bytes([b[0], b[1]]))
        };
        let (source_port, destination_port) = match bytes[9] {
            6 if body.len() >= 20
                && body[12] >> 4 >= 5
                && usize::from(body[12] >> 4) * 4 <= body.len() =>
            {
                (word(0)?, word(2)?)
            }
            17 if body.len() >= 8 && usize::from(word(4)?) == body.len() => (word(0)?, word(2)?),
            1 if body.len() >= 8 && matches!(body[0], 0 | 8) && body[1] == 0 => {
                (word(4)?, word(6)?)
            }
            _ => return None,
        };
        Some(Self {
            source,
            destination,
            protocol: bytes[9],
            source_port,
            destination_port,
        })
    }

    fn reverse(self) -> Self {
        Self {
            source: self.destination,
            destination: self.source,
            source_port: if self.protocol == 1 {
                self.source_port
            } else {
                self.destination_port
            },
            destination_port: if self.protocol == 1 {
                self.destination_port
            } else {
                self.source_port
            },
            ..self
        }
    }

    fn opens(self, bytes: &[u8]) -> bool {
        match self.protocol {
            6 => bytes[33] & 0x17 == 0x02,
            1 => bytes[20] == 8,
            17 => true,
            _ => false,
        }
    }

    fn is_reply(self, bytes: &[u8]) -> bool {
        match self.protocol {
            6 => bytes[33] & 0x02 == 0 || bytes[33] & 0x10 != 0,
            1 => bytes[20] == 0,
            17 => true,
            _ => false,
        }
    }
}

impl ExitPlan {
    pub fn new(
        local: Ipv4Addr,
        mesh: Ipv4Net,
        mtu: u16,
        routes: Vec<ExitRoute>,
        grants: Vec<ExitGrant>,
    ) -> Self {
        Self {
            local,
            mesh,
            mtu,
            specific_routes: routes.clone(),
            direct_prefixes: vec![],
            routes,
            profiles: None,
            selected_profile: None,
            effective_profile: None,
            grants,
            mesh_admission: None,
            flows: Mutex::default(),
            returns: Mutex::default(),
        }
    }

    pub fn with_profiles(
        mut self,
        catalog: ProfileCatalog,
        selected: Option<&str>,
    ) -> Result<Self, AgentError> {
        let mut ids = BTreeSet::new();
        if catalog.profiles.is_empty()
            || catalog.profiles.len() > 16
            || catalog.profiles.iter().any(|p| {
                !meshway_policy::v3::valid_profile_id(&p.id)
                    || p.id == "auto"
                    || !ids.insert(p.id.as_str())
            })
        {
            return Err(AgentError::Invalid("invalid or duplicate routing profile"));
        }
        if (catalog.default_profile == "auto" && catalog.automatic.is_none())
            || (catalog.default_profile != "auto"
                && !ids.contains(catalog.default_profile.as_str()))
        {
            return Err(AgentError::Invalid(
                "default routing profile is not allowed",
            ));
        }
        if let Some(auto) = &catalog.automatic
            && (!ids.contains(auto.fallback_profile.as_str())
                || auto.region_to_profile.len() > 256
                || auto
                    .region_to_profile
                    .values()
                    .any(|id| !ids.contains(id.as_str()))
                || auto
                    .resolved_profile
                    .as_ref()
                    .is_some_and(|id| !ids.contains(id.as_str())))
        {
            return Err(AgentError::Invalid(
                "automatic profile resolution is outside the allowed catalog",
            ));
        }
        // Match the policy compiler: a cached local choice may survive a
        // controller update, but cannot override its newly locked default.
        let selected = if catalog.locked {
            catalog.default_profile.clone()
        } else {
            selected.unwrap_or(&catalog.default_profile).to_owned()
        };
        let effective = if selected == "auto" {
            let auto = catalog
                .automatic
                .as_ref()
                .ok_or(AgentError::Invalid("automatic profile is not allowed"))?;
            auto.resolved_profile
                .as_ref()
                .unwrap_or(&auto.fallback_profile)
                .clone()
        } else {
            selected.clone()
        };
        let profile = catalog
            .profiles
            .iter()
            .find(|p| p.id == effective)
            .ok_or(AgentError::Invalid("routing profile is not authorized"))?;
        self.specific_routes = profile.routes.clone();
        self.routes = profile.routes.clone();
        self.direct_prefixes = profile.direct_prefixes.clone();
        if let ProfileFallback::Exit(route) = &profile.fallback {
            if route.prefix.prefix_len() != 0 {
                return Err(AgentError::Invalid(
                    "profile fallback exit is not a default route",
                ));
            }
            self.routes.push(route.clone());
        }
        self.selected_profile = Some(selected);
        self.effective_profile = Some(effective);
        self.profiles = Some(catalog);
        Ok(self)
    }

    pub fn with_direct(mut self, prefixes: Vec<Ipv4Net>) -> Self {
        self.direct_prefixes = prefixes;
        self
    }

    /// Compose the existing coherent Mesh boundary instead of making the exit
    /// default intercept or grant Mesh destinations. Runtime paths remain explicit.
    pub fn with_mesh(mut self, mesh: &super::PacketPlan) -> Result<Self, AgentError> {
        if mesh.local != self.local || mesh.prefix != self.mesh || mesh.mtu != self.mtu {
            return Err(AgentError::Invalid(
                "exit and Mesh plans have different address bindings",
            ));
        }
        self.mesh_admission = Some(mesh.clone());
        Ok(self)
    }

    /// Fresh flow generation, unchanged policy revision. Retire old I/O before
    /// changing capture and activating the returned plan.
    pub fn select_profile(&self, selected: &str) -> Result<Self, AgentError> {
        let catalog = self
            .profiles
            .as_ref()
            .ok_or(AgentError::Invalid("no allowed routing profiles"))?;
        if catalog.locked && catalog.default_profile != selected {
            return Err(AgentError::Invalid(
                "routing profile selection is administrator locked",
            ));
        }
        let mut next = Self::new(self.local, self.mesh, self.mtu, vec![], self.grants.clone())
            .with_profiles(catalog.clone(), Some(selected))?;
        next.mesh_admission = self.mesh_admission.clone();
        Ok(next)
    }

    /// Trusted controller update, not a local selection/configuration request.
    pub fn resolve_automatic(&self, resolved: Option<&str>) -> Result<Self, AgentError> {
        let mut catalog = self
            .profiles
            .clone()
            .ok_or(AgentError::Invalid("no allowed routing profiles"))?;
        let auto = catalog
            .automatic
            .as_mut()
            .ok_or(AgentError::Invalid("automatic profile is not allowed"))?;
        auto.resolved_profile = resolved.map(str::to_owned);
        let mut next = Self::new(self.local, self.mesh, self.mtu, vec![], self.grants.clone())
            .with_profiles(catalog, self.selected_profile.as_deref())?;
        next.mesh_admission = self.mesh_admission.clone();
        Ok(next)
    }

    pub fn validate(&self, config: &LabConfig) -> Result<(), AgentError> {
        config
            .validate()
            .map_err(|_| AgentError::Invalid("invalid exit runtime configuration"))?;
        let default_capture = self.routes.iter().any(|r| r.prefix.prefix_len() == 0)
            || self.profiles.as_ref().is_some_and(|c| {
                c.profiles
                    .iter()
                    .any(|p| matches!(p.fallback, ProfileFallback::Exit(_)))
            });
        if default_capture
            && (config
                .listeners
                .iter()
                .any(|listener| listener.bind.is_ipv6())
                || config.outgoing.iter().any(|candidate| {
                    candidate
                        .endpoint
                        .parse::<std::net::SocketAddr>()
                        .is_ok_and(|address| address.is_ipv6())
                }))
        {
            return Err(AgentError::Invalid(
                "IPv6 underlay is not supported by the default-exit prototype; socket mark protection is required",
            ));
        }
        let invalid = || AgentError::Invalid("invalid exit projection, prefix or identity binding");
        if !self.mesh.contains(&self.local)
            || self.mesh.prefix_len() == 0
            || !(576..=1400).contains(&self.mtu)
            || self.routes.len() + self.direct_prefixes.len() > 16384
            || self.grants.len() > 64
            || self.grants.iter().map(|g| g.prefixes.len()).sum::<usize>() > 16384
        {
            return Err(invalid());
        }
        let valid_path = |path: &Vec<String>, reverse: &Vec<String>| {
            config.allowed_paths.contains(path)
                && config.allowed_paths.contains(reverse)
                && path.iter().rev().eq(reverse.iter())
                && meshway_dataplane::native_udp_datagram_len(path, usize::from(self.mtu))
                    .is_ok_and(|n| n + 48 <= 1500)
        };
        let mut prefixes = BTreeSet::new();
        for route in &self.routes {
            if route.prefix != route.prefix.trunc()
                || !prefixes.insert(route.prefix)
                || route.path.first() != Some(&config.node_id)
                || !valid_path(&route.path, &route.return_path)
            {
                return Err(invalid());
            }
        }
        for prefix in &self.direct_prefixes {
            if *prefix != prefix.trunc()
                || prefix.prefix_len() == 0
                || !prefixes.insert(*prefix)
                || prefix.contains(&self.mesh.network())
                || self.mesh.contains(&prefix.network())
            {
                return Err(invalid());
            }
        }
        // Controller supplies the flattened first-match result, not raw rules.
        // The default candidate is a final and is intentionally excluded here.
        let mut intervals: Vec<_> = self
            .specific_routes
            .iter()
            .map(|r| r.prefix)
            .chain(self.direct_prefixes.iter().copied())
            .map(|p| (u32::from(p.network()), u32::from(p.broadcast())))
            .collect();
        intervals.sort_unstable();
        if intervals.windows(2).any(|pair| pair[0].1 >= pair[1].0) {
            return Err(AgentError::Invalid(
                "exit table is not a disjoint flattened first-match projection",
            ));
        }
        if let Some(catalog) = &self.profiles {
            for profile in &catalog.profiles {
                if profile.routes.iter().any(|r| r.prefix.prefix_len() == 0) {
                    return Err(AgentError::Invalid(
                        "profile default must be represented as its fallback",
                    ));
                }
                let projected = Self::new(
                    self.local,
                    self.mesh,
                    self.mtu,
                    profile.routes.clone(),
                    vec![],
                )
                .with_direct(profile.direct_prefixes.clone());
                projected.validate(config)?;
                if let ProfileFallback::Exit(candidate) = &profile.fallback
                    && (candidate.prefix.prefix_len() != 0
                        || candidate.path.first() != Some(&config.node_id)
                        || !valid_path(&candidate.path, &candidate.return_path))
                {
                    return Err(invalid());
                }
            }
        }
        let mut sources = BTreeMap::new();
        let mut identities = BTreeMap::new();
        let mut grant_paths = BTreeSet::new();
        for grant in &self.grants {
            if !self.mesh.contains(&grant.source)
                || grant.source == self.local
                || sources
                    .insert(grant.source, grant.path.first())
                    .is_some_and(|old| old != grant.path.first())
                || identities
                    .insert(grant.path.first(), grant.source)
                    .is_some_and(|old| old != grant.source)
                || !grant_paths.insert((grant.source, &grant.path))
                || grant.path.last() != Some(&config.node_id)
                || !valid_path(&grant.path, &grant.return_path)
                || grant.prefixes.is_empty()
                || grant.prefixes.len() > 16384
                || grant.prefixes.iter().any(|p| *p != p.trunc())
                || grant.prefixes.iter().collect::<BTreeSet<_>>().len() != grant.prefixes.len()
                || (!grant.authorization.internet && grant.authorization.networks.is_empty())
                || grant.authorization.networks.len() > 16384
                || grant.authorization.networks.iter().any(|p| *p != p.trunc())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }

    fn external(&self, ip: Ipv4Addr) -> bool {
        !self.mesh.contains(&ip)
            && !ip.is_unspecified()
            && !ip.is_loopback()
            && !ip.is_link_local()
            && !ip.is_multicast()
            && !ip.is_broadcast()
            && ip.octets()[0] != 0
            && ip.octets()[0] < 224
    }

    fn route(&self, ip: Ipv4Addr) -> Option<&ExitRoute> {
        if !self.external(ip) {
            return None;
        }
        let route = self
            .routes
            .iter()
            .filter(|route| route.prefix.contains(&ip))
            .max_by_key(|route| route.prefix.prefix_len())?;
        if self
            .direct_prefixes
            .iter()
            .any(|p| p.contains(&ip) && p.prefix_len() >= route.prefix.prefix_len())
        {
            return None;
        }
        Some(route)
    }

    fn remember(&self, flow: Flow, bytes: &[u8]) -> bool {
        let now = Instant::now();
        let mut flows = self
            .flows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        flows.retain(|_, seen| now.duration_since(*seen) < Duration::from_secs(120));
        if let Some(seen) = flows.get_mut(&flow) {
            *seen = now;
            return true;
        }
        if flows.len() >= 4096 || !flow.opens(bytes) {
            return false;
        }
        flows.insert(flow, now);
        true
    }

    fn reply(&self, flow: Flow, bytes: &[u8]) -> bool {
        if !flow.is_reply(bytes) {
            return false;
        }
        let flows = self
            .flows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        flows
            .get(&flow.reverse())
            .is_some_and(|seen| seen.elapsed() < Duration::from_secs(120))
    }

    fn remember_return(&self, flow: Flow, bytes: &[u8], grant: &ExitGrant) -> bool {
        let now = Instant::now();
        let mut returns = self
            .returns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        returns.retain(|_, (_, seen)| now.duration_since(*seen) < Duration::from_secs(120));
        if let Some((path, seen)) = returns.get_mut(&flow) {
            if *path != grant.return_path && !flow.opens(bytes) {
                return false;
            }
            *path = grant.return_path.clone();
            *seen = now;
            return true;
        }
        if returns.len() >= 4096 || !flow.opens(bytes) {
            return false;
        }
        returns.insert(flow, (grant.return_path.clone(), now));
        true
    }
}

impl Admission for ExitPlan {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        if let Some(path) = self
            .mesh_admission
            .as_ref()
            .and_then(|mesh| mesh.outgoing(bytes))
        {
            return Some(path);
        }
        let flow = Flow::parse(bytes, self.mtu)?;
        if flow.source == self.local {
            let route = self.route(flow.destination)?;
            return self.remember(flow, bytes).then_some(&route.path);
        }
        // E receives only de-NATed established packets from the kernel TUN.
        if !self.external(flow.source) || !flow.is_reply(bytes) {
            return None;
        }
        let returns = self
            .returns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (path, seen) = returns.get(&flow.reverse())?;
        if seen.elapsed() >= Duration::from_secs(120) {
            return None;
        }
        self.grants
            .iter()
            .find(|grant| {
                grant.source == flow.destination
                    && grant.return_path == *path
                    && grant.prefixes.iter().any(|p| p.contains(&flow.source))
                    && grant.authorization.allows(flow.source)
            })
            .map(|grant| &grant.return_path)
    }

    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        if self
            .mesh_admission
            .as_ref()
            .is_some_and(|mesh| mesh.incoming(packet))
        {
            return true;
        }
        if packet.path.first() != Some(&packet.source) {
            return false;
        }
        let Some(flow) = Flow::parse(&packet.payload, self.mtu) else {
            return false;
        };
        if flow.destination == self.local {
            return self
                .route(flow.source)
                .is_some_and(|route| route.return_path == packet.path)
                && self.reply(flow, &packet.payload);
        }
        self.external(flow.destination)
            && self.grants.iter().any(|grant| {
                grant.path == packet.path
                    && grant.source == flow.source
                    && grant.prefixes.iter().any(|p| p.contains(&flow.destination))
                    && grant.authorization.allows(flow.destination)
                    && self.remember_return(flow, &packet.payload, grant)
            })
    }
}

pub(crate) struct ExitAdmission(pub std::sync::Arc<ExitPlan>);

impl Admission for ExitAdmission {
    fn outgoing(&self, bytes: &[u8]) -> Option<&Vec<String>> {
        self.0.outgoing(bytes)
    }
    fn incoming(&self, packet: &ReceivedPacket) -> bool {
        self.0.incoming(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_dataplane::tests::{checksum, packet};

    fn udp(source: &str, destination: &str, sport: u16, dport: u16) -> Vec<u8> {
        let mut p = packet(source, destination, 17);
        p[20..22].copy_from_slice(&sport.to_be_bytes());
        p[22..24].copy_from_slice(&dport.to_be_bytes());
        p[24..26].copy_from_slice(&8u16.to_be_bytes());
        checksum(&mut p);
        p
    }
    fn plan() -> ExitPlan {
        ExitPlan::new(
            "198.19.90.1".parse().unwrap(),
            "198.19.90.0/24".parse().unwrap(),
            1000,
            vec![ExitRoute {
                prefix: "0.0.0.0/0".parse().unwrap(),
                path: vec!["c".into(), "r".into(), "e".into()],
                return_path: vec!["e".into(), "r".into(), "c".into()],
            }],
            vec![],
        )
    }
    #[test]
    fn return_requires_live_flow_identity_exact_path_and_longest_route() {
        let mut p = plan();
        let receive = |payload| ReceivedPacket {
            source: "e".into(),
            path: vec!["e".into(), "r".into(), "c".into()],
            payload,
        };
        let reply = udp("203.0.113.9", "198.19.90.1", 443, 40000);
        assert!(!p.incoming(&receive(reply.clone())));
        assert!(
            p.outgoing(&udp("198.19.90.1", "203.0.113.9", 40000, 443))
                .is_some()
        );
        assert!(p.incoming(&receive(reply.clone())));
        assert!(!p.incoming(&receive(udp("203.0.113.9", "198.19.90.1", 443, 40001))));
        let mut forged = receive(reply.clone());
        forged.source = "r".into();
        assert!(!p.incoming(&forged));
        forged.source = "e".into();
        forged.path = vec!["e".into(), "c".into()];
        assert!(!p.incoming(&forged));
        p.routes.push(ExitRoute {
            prefix: "203.0.113.0/24".parse().unwrap(),
            path: vec!["c".into(), "other".into()],
            return_path: vec!["other".into(), "c".into()],
        });
        assert!(!p.incoming(&receive(reply)));
        p.flows.lock().unwrap().clear();
        assert!(!p.incoming(&receive(udp("203.0.113.9", "198.19.90.1", 443, 40000))));
    }
    #[test]
    fn exit_rejects_spoofed_source_unauthorized_prefix_and_nonmesh_identity() {
        let p = ExitPlan::new(
            "198.19.90.3".parse().unwrap(),
            "198.19.90.0/24".parse().unwrap(),
            1000,
            vec![],
            vec![ExitGrant {
                source: "198.19.90.1".parse().unwrap(),
                path: vec!["c".into(), "r".into(), "e".into()],
                return_path: vec!["e".into(), "r".into(), "c".into()],
                prefixes: vec!["203.0.113.0/24".parse().unwrap()],
                authorization: ExitAuthorization {
                    internet: true,
                    networks: vec![],
                },
            }],
        );
        let receive = |source, destination| ReceivedPacket {
            source: "c".into(),
            path: vec!["c".into(), "r".into(), "e".into()],
            payload: udp(source, destination, 40000, 443),
        };
        assert!(p.incoming(&receive("198.19.90.1", "203.0.113.9")));
        for (s, d) in [
            ("198.19.90.2", "203.0.113.9"),
            ("198.19.90.1", "203.0.114.9"),
            ("10.0.0.1", "203.0.113.9"),
            ("198.19.90.1", "198.19.90.3"),
        ] {
            assert!(!p.incoming(&receive(s, d)));
        }
    }
    #[test]
    fn flow_state_is_bounded_and_idle_expiry_does_not_renew_on_injection() {
        let p = plan();
        for port in 1..=4096 {
            assert!(
                p.outgoing(&udp("198.19.90.1", "203.0.113.9", port, 443))
                    .is_some()
            );
        }
        assert!(
            p.outgoing(&udp("198.19.90.1", "203.0.113.9", 5000, 443))
                .is_none()
        );
        for seen in p.flows.lock().unwrap().values_mut() {
            *seen = Instant::now() - Duration::from_secs(121);
        }
        assert!(
            p.outgoing(&udp("198.19.90.1", "203.0.113.9", 5000, 443))
                .is_some()
        );
        assert_eq!(p.flows.lock().unwrap().len(), 1);
    }

    #[test]
    fn profile_selection_is_allowlisted_locked_and_never_overrides_public_rules_or_direct() {
        let route = |prefix: &str, exit: &str| ExitRoute {
            prefix: prefix.parse().unwrap(),
            path: vec!["c".into(), exit.into()],
            return_path: vec![exit.into(), "c".into()],
        };
        let p = ExitPlan::new(
            "198.19.90.1".parse().unwrap(),
            "198.19.90.0/24".parse().unwrap(),
            1000,
            vec![route("203.0.113.0/24", "e")],
            vec![],
        )
        .with_profiles(
            ProfileCatalog {
                profiles: ["e", "f", "mesh"]
                    .into_iter()
                    .map(|id| RoutingProfile {
                        id: id.into(),
                        routes: vec![route("203.0.113.0/24", "e")],
                        direct_prefixes: vec!["192.0.2.0/24".parse().unwrap()],
                        fallback: if id == "mesh" {
                            ProfileFallback::Direct
                        } else {
                            ProfileFallback::Exit(route("0.0.0.0/0", id))
                        },
                    })
                    .collect(),
                default_profile: "e".into(),
                locked: false,
                automatic: Some(AutomaticProfile {
                    region_to_profile: BTreeMap::from([
                        ("US".into(), "e".into()),
                        ("JP".into(), "f".into()),
                    ]),
                    resolved_profile: Some("e".into()),
                    fallback_profile: "mesh".into(),
                }),
            },
            None,
        )
        .unwrap();
        assert!(p.select_profile("unauthorized").is_err());
        let next = p.select_profile("f").unwrap();
        assert_eq!(
            next.outgoing(&udp("198.19.90.1", "203.0.113.1", 1000, 443))
                .unwrap()
                .last()
                .unwrap(),
            "e"
        );
        assert_eq!(
            next.outgoing(&udp("198.19.90.1", "203.0.114.1", 1000, 443))
                .unwrap()
                .last()
                .unwrap(),
            "f"
        );
        assert!(
            next.outgoing(&udp("198.19.90.1", "192.0.2.1", 1000, 443))
                .is_none()
        );
        let mut none = next.select_profile("mesh").unwrap();
        assert!(
            none.outgoing(&udp("198.19.90.1", "203.0.114.1", 1000, 443))
                .is_none()
        );
        assert!(
            none.outgoing(&udp("198.19.90.1", "203.0.113.1", 1000, 443))
                .is_some()
        );
        let automatic = next
            .select_profile("auto")
            .unwrap()
            .resolve_automatic(Some("f"))
            .unwrap();
        assert_eq!(automatic.selected_profile.as_deref(), Some("auto"));
        assert_eq!(automatic.effective_profile.as_deref(), Some("f"));
        assert_eq!(
            automatic
                .resolve_automatic(None)
                .unwrap()
                .effective_profile
                .as_deref(),
            Some("mesh")
        );
        assert!(automatic.resolve_automatic(Some("unauthorized")).is_err());
        none.profiles.as_mut().unwrap().locked = true;
        none.profiles.as_mut().unwrap().default_profile = "mesh".into();
        assert!(none.select_profile("e").is_err());
        assert!(none.select_profile("mesh").is_ok());
    }

    #[test]
    fn raw_overlapping_rules_and_unprotected_ipv6_default_underlay_fail_prepare() {
        let local = meshway_dataplane::Identity::generate().unwrap();
        let remote = meshway_dataplane::Identity::generate().unwrap();
        let path = vec!["c".into(), "e".into()];
        let reverse = vec!["e".into(), "c".into()];
        let mut config = LabConfig {
            node_id: "c".into(),
            private_key: meshway_dataplane::PrivateKey::from_hex(&local.private_key_hex()).unwrap(),
            public_key: local.public_key_hex(),
            trusted_peers: vec![meshway_dataplane::PeerConfig {
                node_id: "e".into(),
                public_key: remote.public_key_hex(),
                transports: vec![meshway_dataplane::TransportKind::NativeUdp],
            }],
            allowed_paths: vec![path.clone(), reverse.clone()],
            listeners: vec![],
            outgoing: vec![],
            limits: Default::default(),
        };
        let route = |prefix: &str| ExitRoute {
            prefix: prefix.parse().unwrap(),
            path: path.clone(),
            return_path: reverse.clone(),
        };
        let projection = |routes| {
            ExitPlan::new(
                "198.19.90.1".parse().unwrap(),
                "198.19.90.0/24".parse().unwrap(),
                1000,
                routes,
                vec![],
            )
        };
        assert!(
            projection(vec![route("203.0.113.0/24"), route("203.0.113.0/25")])
                .validate(&config)
                .is_err()
        );
        let default = projection(vec![])
            .with_profiles(
                ProfileCatalog {
                    profiles: vec![RoutingProfile {
                        id: "global".into(),
                        routes: vec![],
                        direct_prefixes: vec![],
                        fallback: ProfileFallback::Exit(route("0.0.0.0/0")),
                    }],
                    default_profile: "global".into(),
                    locked: false,
                    automatic: None,
                },
                None,
            )
            .unwrap();
        assert!(default.validate(&config).is_ok());
        config.outgoing.push(meshway_dataplane::OutgoingCandidate {
            peer: "e".into(),
            transport: meshway_dataplane::TransportKind::NativeUdp,
            endpoint: "[2001:db8::1]:41170".into(),
            priority: 0,
            id: None,
        });
        assert!(default.validate(&config).is_err());
    }

    #[test]
    fn exit_return_tracks_actual_authorized_forward_path_not_first_grant() {
        let direct = vec!["c".into(), "e".into()];
        let relayed = vec!["c".into(), "r".into(), "e".into()];
        let grants = [direct.clone(), relayed.clone()]
            .into_iter()
            .map(|path: Vec<String>| ExitGrant {
                source: "198.19.90.1".parse().unwrap(),
                return_path: path.iter().rev().cloned().collect(),
                path,
                prefixes: vec!["203.0.113.0/24".parse().unwrap()],
                authorization: ExitAuthorization {
                    internet: true,
                    networks: vec![],
                },
            })
            .collect();
        let p = ExitPlan::new(
            "198.19.90.3".parse().unwrap(),
            "198.19.90.0/24".parse().unwrap(),
            1000,
            vec![],
            grants,
        );
        let reply = udp("203.0.113.9", "198.19.90.1", 443, 40000);
        assert!(
            p.outgoing(&reply).is_none(),
            "no userspace ingress association"
        );
        assert!(p.incoming(&ReceivedPacket {
            source: "c".into(),
            path: relayed.clone(),
            payload: udp("198.19.90.1", "203.0.113.9", 40000, 443)
        }));
        assert_eq!(
            p.outgoing(&reply).unwrap(),
            &relayed.into_iter().rev().collect::<Vec<_>>()
        );
        let other_reply = udp("203.0.113.9", "198.19.90.1", 443, 40001);
        assert!(p.incoming(&ReceivedPacket {
            source: "c".into(),
            path: direct.clone(),
            payload: udp("198.19.90.1", "203.0.113.9", 40001, 443)
        }));
        assert_eq!(
            p.outgoing(&other_reply).unwrap(),
            &direct.into_iter().rev().collect::<Vec<_>>()
        );
    }

    #[test]
    fn internet_does_not_authorize_private_cgnat_or_link_local_destinations() {
        let scope = ExitAuthorization {
            internet: true,
            networks: vec![],
        };
        for address in [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "169.254.1.1",
        ] {
            assert!(!scope.allows(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "172.15.255.254", "172.32.0.1", "100.128.0.1"] {
            assert!(scope.allows(address.parse().unwrap()));
        }
        let mut grant = ExitGrant {
            source: "198.19.90.1".parse().unwrap(),
            path: vec!["c".into(), "e".into()],
            return_path: vec!["e".into(), "c".into()],
            prefixes: vec!["0.0.0.0/0".parse().unwrap()],
            authorization: scope,
        };
        let incoming = ReceivedPacket {
            source: "c".into(),
            path: grant.path.clone(),
            payload: udp("198.19.90.1", "192.168.1.9", 40000, 443),
        };
        let make = |grant| {
            ExitPlan::new(
                "198.19.90.3".parse().unwrap(),
                "198.19.90.0/24".parse().unwrap(),
                1000,
                vec![],
                vec![grant],
            )
        };
        assert!(!make(grant.clone()).incoming(&incoming));
        grant
            .authorization
            .networks
            .push("192.168.1.0/24".parse().unwrap());
        assert!(make(grant).incoming(&incoming));
    }
}
