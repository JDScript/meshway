use crate::error::Error;
use chrono::{DateTime, Utc};
use maxminddb::{Reader, geoip2};
use meshway_model::{ArtifactId, resources::*, state::EndpointSource};
use meshway_protocol::state::{DerivedNodeState, VisibleReport};
use std::{collections::BTreeMap, net::IpAddr};

#[derive(Default)]
pub struct GeoIp {
    city: Option<(ArtifactId, Reader<Vec<u8>>)>,
    asn: Option<(ArtifactId, Reader<Vec<u8>>)>,
    cache: BTreeMap<IpAddr, IpInfo>,
}
impl GeoIp {
    pub fn dataset_ids(&self) -> (Option<ArtifactId>, Option<ArtifactId>) {
        (
            self.city.as_ref().map(|(id, _)| *id),
            self.asn.as_ref().map(|(id, _)| *id),
        )
    }
    /// Deployment-owned bytes, loaded from immutable artifacts. Replacing a
    /// dataset clears derived cache, never modifies source report timestamps.
    pub fn install_city(&mut self, id: ArtifactId, bytes: Vec<u8>) -> Result<(), Error> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err(Error::StateLimit);
        }
        let reader = Reader::from_source(bytes).map_err(|_| Error::InvalidRequest)?;
        if !reader.metadata.database_type.contains("City")
            && !reader.metadata.database_type.contains("Country")
        {
            return Err(Error::InvalidRequest);
        }
        self.city = Some((id, reader));
        self.cache.clear();
        Ok(())
    }
    pub fn install_asn(&mut self, id: ArtifactId, bytes: Vec<u8>) -> Result<(), Error> {
        if bytes.len() > 128 * 1024 * 1024 {
            return Err(Error::StateLimit);
        }
        let reader = Reader::from_source(bytes).map_err(|_| Error::InvalidRequest)?;
        if !reader.metadata.database_type.contains("ASN") {
            return Err(Error::InvalidRequest);
        }
        self.asn = Some((id, reader));
        self.cache.clear();
        Ok(())
    }
    pub fn lookup(&mut self, ip: IpAddr, now: DateTime<Utc>) -> IpInfo {
        if !is_public(ip) {
            return IpInfo {
                geoip: LookupResult::NotApplicable {
                    reason: NotApplicableReason::NonPublicAddress,
                },
                asn: LookupResult::NotApplicable {
                    reason: NotApplicableReason::NonPublicAddress,
                },
            };
        }
        if let Some(value) = self.cache.get(&ip) {
            return value.clone();
        }
        let value = IpInfo {
            geoip: self.lookup_city(ip, now),
            asn: self.lookup_asn(ip, now),
        };
        if self.cache.len() >= 8192 {
            self.cache.clear();
        }
        self.cache.insert(ip, value.clone());
        value
    }
    fn lookup_city(&self, ip: IpAddr, now: DateTime<Utc>) -> LookupResult<GeoIpData> {
        match &self.city {
            None => LookupResult::Unavailable {
                reason: UnavailableReason::DatasetUnavailable,
            },
            Some((id, reader)) => match reader.lookup::<geoip2::City<'_>>(ip) {
                Ok(Some(city)) => {
                    let position = city
                        .location
                        .and_then(|location| {
                            Some(GeoPosition::new(
                                location.latitude?,
                                location.longitude?,
                                location.accuracy_radius.map(f64::from),
                            ))
                        })
                        .transpose();
                    let Ok(position) = position else {
                        return LookupResult::Unavailable {
                            reason: UnavailableReason::LookupFailed,
                        };
                    };
                    LookupResult::Available {
                        dataset_id: *id,
                        looked_up_at: now,
                        data: GeoIpData {
                            country: city.country.and_then(|c| c.iso_code.map(str::to_owned)),
                            region: city
                                .subdivisions
                                .and_then(|s| s.into_iter().next())
                                .and_then(|s| {
                                    s.names.and_then(|names| {
                                        names.get("en").copied().map(str::to_owned)
                                    })
                                }),
                            city: city.city.and_then(|city| {
                                city.names
                                    .and_then(|names| names.get("en").copied().map(str::to_owned))
                            }),
                            position,
                        },
                    }
                }
                Ok(None) => LookupResult::NotFound {
                    dataset_id: *id,
                    looked_up_at: now,
                },
                Err(_) => LookupResult::Unavailable {
                    reason: UnavailableReason::LookupFailed,
                },
            },
        }
    }
    fn lookup_asn(&self, ip: IpAddr, now: DateTime<Utc>) -> LookupResult<AsnData> {
        match &self.asn {
            None => LookupResult::Unavailable {
                reason: UnavailableReason::DatasetUnavailable,
            },
            Some((id, reader)) => match reader.lookup::<geoip2::Asn<'_>>(ip) {
                Ok(Some(asn)) => match asn.autonomous_system_number {
                    Some(number) if number != 0 => LookupResult::Available {
                        dataset_id: *id,
                        looked_up_at: now,
                        data: AsnData {
                            number,
                            organization: asn.autonomous_system_organization.map(str::to_owned),
                            prefix: None,
                        },
                    },
                    _ => LookupResult::NotFound {
                        dataset_id: *id,
                        looked_up_at: now,
                    },
                },
                Ok(None) => LookupResult::NotFound {
                    dataset_id: *id,
                    looked_up_at: now,
                },
                Err(_) => LookupResult::Unavailable {
                    reason: UnavailableReason::LookupFailed,
                },
            },
        }
    }
    pub fn derive(
        &mut self,
        report: Option<&VisibleReport>,
        current: bool,
        mesh_addresses: &std::collections::BTreeSet<IpAddr>,
        now: DateTime<Utc>,
    ) -> DerivedNodeState {
        let mut endpoints = vec![];
        match report {
            Some(VisibleReport::Full { report }) => {
                for (socket, state) in &report.sockets {
                    for endpoint in &state.endpoints {
                        let source = match endpoint.origin {
                            EndpointSource::Configured { .. } => None,
                            EndpointSource::Interface { .. } => Some(LocationSource::Interface),
                            EndpointSource::Stun { .. } => Some(LocationSource::Stun),
                        };
                        endpoints.push((
                            socket,
                            state.generation,
                            &endpoint.address,
                            source,
                            endpoint.observed_at,
                            endpoint.expires_at,
                        ));
                    }
                }
            }
            Some(VisibleReport::Peer { report }) => {
                for endpoint in &report.endpoints {
                    let source = match endpoint.candidate.source {
                        meshway_protocol::state::CandidateSource::Configured => None,
                        meshway_protocol::state::CandidateSource::Interface => {
                            Some(LocationSource::Interface)
                        }
                        meshway_protocol::state::CandidateSource::Stun => {
                            Some(LocationSource::Stun)
                        }
                    };
                    endpoints.push((
                        &endpoint.socket_id,
                        endpoint.generation,
                        &endpoint.candidate.address,
                        source,
                        endpoint.candidate.observed_at,
                        endpoint.candidate.expires_at,
                    ));
                }
            }
            None => {}
        }
        let mut ip_info = BTreeMap::new();
        let mut observations = vec![];
        let mut observed_ips = std::collections::BTreeSet::new();
        let mut absent = LocationAbsentReason::NoObservation;
        for (socket, generation, address, source, observed_at, expires_at) in endpoints {
            let Ok(address) = address.parse::<std::net::SocketAddr>() else {
                continue;
            };
            let info = if mesh_addresses.contains(&address.ip()) {
                IpInfo {
                    geoip: LookupResult::NotApplicable {
                        reason: NotApplicableReason::MeshAddress,
                    },
                    asn: LookupResult::NotApplicable {
                        reason: NotApplicableReason::MeshAddress,
                    },
                }
            } else {
                self.lookup(address.ip(), now)
            };
            ip_info.insert(address.ip().to_string(), info.clone());
            let Some(source) = source else {
                continue;
            };
            if !is_public(address.ip()) || mesh_addresses.contains(&address.ip()) {
                continue;
            }
            if !current || expires_at <= now {
                absent = LocationAbsentReason::ObservationExpired;
                continue;
            }
            observed_ips.insert(address.ip());
            match info.geoip {
                LookupResult::Available {
                    dataset_id, data, ..
                } => observations.push(GeoObservation {
                    address: address.ip().to_string(),
                    source_socket_id: socket.clone(),
                    source_generation: generation,
                    dataset_id,
                    data,
                    endpoint_source: source,
                    observed_at,
                    expires_at,
                }),
                LookupResult::NotFound { .. } => absent = LocationAbsentReason::NotInDataset,
                LookupResult::Unavailable {
                    reason: UnavailableReason::DatasetUnavailable,
                } => absent = LocationAbsentReason::DatasetUnavailable,
                _ => absent = LocationAbsentReason::LookupFailed,
            }
        }
        // No preferred-underlay decision exists. Even two successful different
        // public IP observations are ambiguous rather than map-order selection.
        let different = observed_ips.len() > 1;
        let location = if different {
            absent = LocationAbsentReason::AmbiguousObservation;
            None
        } else {
            observations.into_iter().min_by_key(|o| o.expires_at)
        };
        DerivedNodeState {
            ip_info,
            location_absent: if location.is_none() {
                Some(absent)
            } else {
                None
            },
            location,
        }
    }
}
pub(super) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || a == 100 && (64..=127).contains(&b)
                || a == 198 && (b == 18 || b == 19)
                || a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            ip.to_ipv4_mapped().is_none()
                && s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] == 0xdb8 || s[1] < 0x200))
        }
    }
}
