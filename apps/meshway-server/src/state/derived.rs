//! Address facts derived from the endpoints a caller may see. No geographic
//! lookup happens here; IP datasets serve only automatic routing profiles.
use chrono::{DateTime, Utc};
use meshway_model::{
    Sequence,
    resources::{AddressScope, LocationAbsentReason, LocationObservation, LocationSource},
    state::EndpointSource,
};
use meshway_protocol::state::{CandidateSource, DerivedNodeState, VisibleReport};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, SocketAddr},
};

struct Observed<'a> {
    socket: &'a str,
    generation: Sequence,
    address: &'a str,
    source: Option<LocationSource>,
    observed_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

/// Eligible observations of one rank, by address.
type ByAddress<'a> = BTreeMap<IpAddr, Vec<(LocationSource, &'a Observed<'a>)>>;

/// `current` is whether the report itself still supports current facts.
///
/// The location address is chosen by rank: STUN before interface, then IPv4
/// before IPv6. Exactly one distinct address at the best present rank is the
/// location; more than one is `ambiguousObservation`. Among observations of
/// that address, the latest deadline wins, then the smallest socket ID.
pub(crate) fn derive(
    report: Option<&VisibleReport>,
    current: bool,
    mesh_addresses: &BTreeSet<IpAddr>,
    now: DateTime<Utc>,
) -> DerivedNodeState {
    let mut endpoints = vec![];
    match report {
        Some(VisibleReport::Full { report }) => {
            for (socket, state) in &report.sockets {
                for endpoint in &state.endpoints {
                    endpoints.push(Observed {
                        socket,
                        generation: state.generation,
                        address: &endpoint.address,
                        source: match endpoint.origin {
                            EndpointSource::Configured { .. } => None,
                            EndpointSource::Interface { .. } => Some(LocationSource::Interface),
                            EndpointSource::Stun { .. } => Some(LocationSource::Stun),
                        },
                        observed_at: endpoint.observed_at,
                        expires_at: endpoint.expires_at,
                    });
                }
            }
        }
        Some(VisibleReport::Peer { report }) => {
            for endpoint in &report.endpoints {
                endpoints.push(Observed {
                    socket: &endpoint.socket_id,
                    generation: endpoint.generation,
                    address: &endpoint.candidate.address,
                    source: match endpoint.candidate.source {
                        CandidateSource::Configured => None,
                        CandidateSource::Interface => Some(LocationSource::Interface),
                        CandidateSource::Stun => Some(LocationSource::Stun),
                    },
                    observed_at: endpoint.candidate.observed_at,
                    expires_at: endpoint.candidate.expires_at,
                });
            }
        }
        None => {}
    }
    let mut address_scope = BTreeMap::new();
    let mut eligible: BTreeMap<(u8, u8), ByAddress<'_>> = BTreeMap::new();
    let mut expired = false;
    for endpoint in &endpoints {
        // Domain endpoints carry no observed IP.
        let Ok(address) = endpoint.address.parse::<SocketAddr>() else {
            continue;
        };
        let ip = address.ip();
        let scope = if mesh_addresses.contains(&ip) {
            AddressScope::Mesh
        } else if crate::resources::is_public(ip) {
            AddressScope::Public
        } else {
            AddressScope::NonPublic
        };
        address_scope.insert(ip.to_string(), scope);
        let Some(source) = endpoint.source else {
            continue;
        };
        if scope != AddressScope::Public {
            continue;
        }
        if !current || endpoint.expires_at <= now {
            expired = true;
            continue;
        }
        let rank = (
            match source {
                LocationSource::Stun => 0,
                LocationSource::Interface => 1,
            },
            u8::from(ip.is_ipv6()),
        );
        eligible
            .entry(rank)
            .or_default()
            .entry(ip)
            .or_default()
            .push((source, endpoint));
    }
    let (location, location_absent) = match eligible.into_values().next() {
        None if expired => (None, Some(LocationAbsentReason::ObservationExpired)),
        None => (None, Some(LocationAbsentReason::NoObservation)),
        Some(addresses) if addresses.len() > 1 => {
            (None, Some(LocationAbsentReason::AmbiguousObservation))
        }
        Some(addresses) => {
            let chosen = addresses.into_iter().next().and_then(|(ip, observations)| {
                observations
                    .into_iter()
                    .max_by(|(_, a), (_, b)| {
                        a.expires_at
                            .cmp(&b.expires_at)
                            .then_with(|| b.socket.cmp(a.socket))
                    })
                    .map(|o| (ip, o))
            });
            match chosen {
                Some((ip, (source, observed))) => (
                    Some(LocationObservation {
                        address: ip.to_string(),
                        source_socket_id: observed.socket.to_owned(),
                        source_generation: observed.generation,
                        endpoint_source: source,
                        observed_at: observed.observed_at,
                        expires_at: observed.expires_at,
                    }),
                    None,
                ),
                None => (None, Some(LocationAbsentReason::NoObservation)),
            }
        }
    };
    DerivedNodeState {
        address_scope,
        location,
        location_absent,
    }
}
