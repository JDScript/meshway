//! Exact lookup in the sole candidate store; no traversal candidate cache.
use super::StateHub;
use crate::{
    error::Error,
    execution::connectivity::{Failure, Scope, carrier_resources},
};
use chrono::{DateTime, Utc};
use meshway_configuration::execution::Carrier;
use meshway_model::state::{Capability, ResourceRef, SocketNetwork, StreamKind, TransportRole};
use meshway_protocol::connectivity::{CandidateReference, UdpTraversalUnavailableReason as Reason};
use meshway_transport_spec::Listener;
use std::net::SocketAddr;

pub(crate) struct Candidate {
    pub address: String,
    pub target: SocketAddr,
    pub deadline_ms: u64,
    pub source: meshway_model::observations::EndpointKind,
}

impl StateHub {
    pub(crate) fn traversal_candidate(
        &self,
        scope: &Scope,
        carrier: &Carrier,
        reference: &CandidateReference,
        now: DateTime<Utc>,
    ) -> Result<Candidate, Failure> {
        let binding = &reference.socket;
        if binding.node_id != scope.report.node_id
            || binding.instance.instance_id != scope.report.instance_id
            || binding.instance.generation != scope.report.instance_generation
            || Some(&binding.netmap) != scope.report.execution.as_ref()
        {
            return Err(Reason::ScopeChanged.into());
        }
        if !carrier_resources(&scope.target.netmap, carrier).contains(&binding.resource) {
            return Err(Reason::NotAvailable.into());
        }
        let needed = if binding.node_id == carrier.dialer {
            TransportRole::Dial
        } else {
            TransportRole::Listen
        };
        if !matches!(scope.target.netmap.self_.self_keys.capabilities.get("transport:native_udp"),
            Some(Capability::Transport { roles, streams }) if roles.contains(&needed) && streams.contains(&StreamKind::Udp)
                && (!matches!(binding.resource, ResourceRef::Listener { .. }) || roles.contains(&TransportRole::Listen)))
        {
            return Err(Reason::NotAvailable.into());
        }
        let stored = self
            .reports
            .get(&(scope.report.network_id, binding.node_id))
            .ok_or(Reason::CandidateChanged)?;
        if stored.report.instance_id != binding.instance.instance_id
            || stored.instance_generation != binding.instance.generation
            || stored.expires_at <= now
            || stored.report.state.configuration.applied.as_ref() != Some(&binding.netmap)
        {
            return Err(Reason::CandidateChanged.into());
        }
        let socket = stored
            .report
            .state
            .sockets
            .get(&binding.socket_id)
            .ok_or(Reason::CandidateChanged)?;
        if socket.generation != binding.socket_generation
            || socket.generation.get() == 0
            || socket.resource != binding.resource
            || socket.network != SocketNetwork::Udp
            || socket
                .netmap
                .as_ref()
                .or(stored.report.state.configuration.applied.as_ref())
                != Some(&binding.netmap)
        {
            return Err(Reason::CandidateChanged.into());
        }
        if let ResourceRef::Listener { key } = &binding.resource {
            let declaration = scope
                .target
                .netmap
                .self_
                .listeners
                .get(key)
                .ok_or(Reason::NotAvailable)?;
            let Listener::NativeUdp(listener) = declaration.declaration() else {
                return Err(Reason::NotAvailable.into());
            };
            if !listener.enabled
                || socket.bound_address.port() != listener.listen_port.get()
                || socket.bound_address.is_ipv6() != listener.listen.is_ipv6()
                || (!listener.listen.is_unspecified()
                    && socket.bound_address.ip() != listener.listen)
                || stored
                    .report
                    .state
                    .listeners
                    .get(key)
                    .is_none_or(|listener| {
                        listener.phase != meshway_model::state::ListenerPhase::Listening
                            || !listener.socket_ids.contains(&binding.socket_id)
                            || listener.netmap.as_ref().or(stored
                                .report
                                .state
                                .configuration
                                .applied
                                .as_ref())
                                != Some(&binding.netmap)
                    })
            {
                return Err(Reason::CandidateChanged.into());
            }
        }
        for candidate in &socket.endpoints {
            let digest = meshway_protocol::candidate_observation_digest(
                binding,
                socket.network,
                socket.bound_address,
                candidate,
            )
            .map_err(|_| Error::encode("candidate observation"))?;
            if digest != reference.observation_digest {
                continue;
            }
            if candidate.expires_at <= now || candidate.observed_at > now {
                return Err(Reason::CandidateChanged.into());
            }
            let target = eligible_address(
                &candidate.address,
                &scope.target.netmap.self_.mesh_address_pools,
            )?;
            if target.is_ipv6() != socket.bound_address.is_ipv6() {
                return Err(Reason::NotAvailable.into());
            }
            let deadline_ms = u64::try_from(
                candidate
                    .expires_at
                    .min(stored.expires_at)
                    .timestamp_millis(),
            )
            .map_err(|_| Error::decode("candidate deadline"))?;
            return Ok(Candidate {
                address: candidate.address.clone(),
                target,
                deadline_ms,
                source: match &candidate.origin {
                    meshway_model::state::EndpointSource::Configured { .. } => {
                        meshway_model::observations::EndpointKind::Configured
                    }
                    meshway_model::state::EndpointSource::Interface { .. } => {
                        meshway_model::observations::EndpointKind::Interface
                    }
                    meshway_model::state::EndpointSource::Stun { .. } => {
                        meshway_model::observations::EndpointKind::Stun
                    }
                },
            });
        }
        Err(Reason::CandidateChanged.into())
    }
}

pub(crate) fn eligible_address(
    value: &str,
    pools: &meshway_configuration::AddressPools,
) -> Result<SocketAddr, Failure> {
    let address: SocketAddr = value.parse().map_err(|_| Reason::NotAvailable)?;
    let ip = address.ip();
    if address.port() == 0
        || ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || match address {
            SocketAddr::V4(v4) => v4.ip().is_broadcast() || v4.ip().is_link_local(),
            SocketAddr::V6(v6) => {
                v6.scope_id() != 0
                    || v6.flowinfo() != 0
                    || v6.ip().is_unicast_link_local()
                    || v6.ip().to_ipv4_mapped().is_some()
            }
        }
    {
        return Err(Reason::NotAvailable.into());
    }
    for pool in pools.ipv4.iter().chain(&pools.ipv6) {
        let pool: ipnet::IpNet = pool
            .parse()
            .map_err(|_| Error::decode("mesh address pool"))?;
        if pool.contains(&ip) {
            return Err(Reason::NotAvailable.into());
        }
    }
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numeric_unicast_targets_exclude_scopes_aliases_and_complete_mesh_pools() {
        let pools = meshway_configuration::AddressPools {
            ipv4: vec!["100.64.0.0/10".into()],
            ipv6: vec!["fd00::/64".into()],
        };
        for value in [
            "name.example:1234",
            "127.0.0.1:1",
            "[::1]:1",
            "0.0.0.0:1",
            "[::]:1",
            "224.0.0.1:1",
            "[ff02::1]:1",
            "255.255.255.255:1",
            "169.254.1.1:1",
            "[fe80::1]:1",
            "[2001:db8::1%2]:1",
            "[::ffff:192.0.2.1]:1",
            "192.0.2.1:0",
            "100.100.100.100:1",
            "[fd00::1234]:1",
        ] {
            assert!(eligible_address(value, &pools).is_err(), "{value}");
        }
        for value in [
            "192.168.1.2:1000",
            "10.0.0.1:2000",
            "172.16.1.2:3000",
            "198.51.100.1:1234",
            "[2001:db8::1]:1234",
        ] {
            assert!(eligible_address(value, &pools).is_ok(), "{value}");
        }
    }
}
