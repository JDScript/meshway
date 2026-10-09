//! Canonical inner-service sets authorized by one grant.
use crate::{execution::BusinessProtocol, object::object};
use meshway_transport_spec::Port;

object! {
/// Inclusive destination-port interval.
pub struct PortRange {
    /// First destination port in the interval.
    pub start: Port,
    /// Last destination port in the interval, not below start.
    pub end: Port,
} }

object! {
/// One protocol of a service set; ICMP echo has no ports.
pub struct Service {
    /// Inner business protocol.
    pub protocol: BusinessProtocol,
    /// Sorted, disjoint and non-adjacent destination-port intervals; empty only for ICMP echo.
    pub ports: Vec<PortRange>,
} }

/// Ordered, mutually exclusive protocol and port ranges. Exactly one
/// canonical encoding exists for each set, so its digest is order-free.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(transparent)]
pub struct ServiceSet(
    /// Entries ordered tcp, udp, icmpEcho, each protocol at most once.
    Vec<Service>,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("service set is not in canonical form")]
pub struct InvalidServiceSet;

fn rank(protocol: BusinessProtocol) -> u8 {
    match protocol {
        BusinessProtocol::Tcp => 0,
        BusinessProtocol::Udp => 1,
        BusinessProtocol::IcmpEcho => 2,
    }
}

impl ServiceSet {
    /// Union of grants; `None` ports mean ICMP echo, which ignores its range.
    pub fn from_grants(
        grants: impl IntoIterator<Item = (BusinessProtocol, Option<(u16, u16)>)>,
    ) -> Self {
        let mut ranges: [Vec<(u16, u16)>; 2] = [Vec::new(), Vec::new()];
        let mut icmp = false;
        for (protocol, range) in grants {
            match (protocol, range) {
                (BusinessProtocol::IcmpEcho, _) => icmp = true,
                (protocol, Some((start, end))) if start >= 1 && start <= end => {
                    ranges[rank(protocol) as usize].push((start, end));
                }
                _ => {}
            }
        }
        let mut services = Vec::new();
        for (index, protocol) in [BusinessProtocol::Tcp, BusinessProtocol::Udp]
            .into_iter()
            .enumerate()
        {
            let list = &mut ranges[index];
            if list.is_empty() {
                continue;
            }
            list.sort_unstable();
            let mut merged: Vec<(u16, u16)> = Vec::new();
            for (start, end) in list.drain(..) {
                match merged.last_mut() {
                    Some(last) if u32::from(start) <= u32::from(last.1) + 1 => {
                        last.1 = last.1.max(end);
                    }
                    _ => merged.push((start, end)),
                }
            }
            services.push(Service {
                protocol,
                ports: merged
                    .into_iter()
                    .map(|(start, end)| PortRange {
                        start: Port::new(start).expect("nonzero start"),
                        end: Port::new(end).expect("end at least start"),
                    })
                    .collect(),
            });
        }
        if icmp {
            services.push(Service {
                protocol: BusinessProtocol::IcmpEcho,
                ports: Vec::new(),
            });
        }
        Self(services)
    }

    pub fn services(&self) -> &[Service] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Exact inner protocol/port membership; ICMP echo carries no port.
    pub fn contains(&self, protocol: BusinessProtocol, port: Option<Port>) -> bool {
        self.0
            .iter()
            .find(|service| service.protocol == protocol)
            .is_some_and(|service| match (protocol, port) {
                (BusinessProtocol::IcmpEcho, None) => true,
                (BusinessProtocol::Tcp | BusinessProtocol::Udp, Some(port)) => {
                    let index = service.ports.partition_point(|range| range.end < port);
                    service.ports.get(index).is_some_and(|r| r.start <= port)
                }
                _ => false,
            })
    }

    /// Whether every service of `other` is also in this set.
    pub fn covers(&self, other: &ServiceSet) -> bool {
        other.0.iter().all(|service| match service.protocol {
            BusinessProtocol::IcmpEcho => self.contains(BusinessProtocol::IcmpEcho, None),
            protocol => service.ports.iter().all(|range| {
                self.0
                    .iter()
                    .find(|own| own.protocol == protocol)
                    .is_some_and(|own| {
                        own.ports
                            .iter()
                            .any(|r| r.start <= range.start && range.end <= r.end)
                    })
            }),
        })
    }

    fn validate(&self) -> Result<(), InvalidServiceSet> {
        let mut previous = None;
        for service in &self.0 {
            let current = rank(service.protocol);
            if previous.is_some_and(|previous| previous >= current) {
                return Err(InvalidServiceSet);
            }
            previous = Some(current);
            match service.protocol {
                BusinessProtocol::IcmpEcho if !service.ports.is_empty() => {
                    return Err(InvalidServiceSet);
                }
                BusinessProtocol::Tcp | BusinessProtocol::Udp if service.ports.is_empty() => {
                    return Err(InvalidServiceSet);
                }
                _ => {}
            }
            let mut last: Option<u16> = None;
            for range in &service.ports {
                if range.start > range.end
                    || last.is_some_and(|last| u32::from(range.start.get()) <= u32::from(last) + 1)
                {
                    return Err(InvalidServiceSet);
                }
                last = Some(range.end.get());
            }
        }
        Ok(())
    }
}

impl<'de> serde::Deserialize<'de> for ServiceSet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let set = Self(Vec::<Service>::deserialize(d)?);
        set.validate().map_err(serde::de::Error::custom)?;
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(value: u16) -> Option<Port> {
        Some(Port::new(value).unwrap())
    }

    #[test]
    fn union_is_canonical_and_order_free() {
        let a = ServiceSet::from_grants([
            (BusinessProtocol::Udp, Some((53, 53))),
            (BusinessProtocol::Tcp, Some((80, 90))),
            (BusinessProtocol::Tcp, Some((91, 100))),
            (BusinessProtocol::IcmpEcho, None),
            (BusinessProtocol::Tcp, Some((443, 443))),
        ]);
        let b = ServiceSet::from_grants([
            (BusinessProtocol::Tcp, Some((443, 443))),
            (BusinessProtocol::IcmpEcho, Some((1, 1))),
            (BusinessProtocol::Tcp, Some((85, 100))),
            (BusinessProtocol::Tcp, Some((80, 84))),
            (BusinessProtocol::Udp, Some((53, 53))),
        ]);
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            r#"[{"protocol":"tcp","ports":[{"start":80,"end":100},{"start":443,"end":443}]},{"protocol":"udp","ports":[{"start":53,"end":53}]},{"protocol":"icmpEcho","ports":[]}]"#
        );
        assert!(a.contains(BusinessProtocol::Tcp, port(100)));
        assert!(!a.contains(BusinessProtocol::Tcp, port(101)));
        assert!(!a.contains(BusinessProtocol::Udp, port(443)));
        assert!(a.contains(BusinessProtocol::IcmpEcho, None));
        assert!(!a.contains(BusinessProtocol::Tcp, None));
        assert!(a.covers(&ServiceSet::from_grants([(
            BusinessProtocol::Tcp,
            Some((81, 99))
        )])));
        assert!(!a.covers(&ServiceSet::from_grants([(
            BusinessProtocol::Tcp,
            Some((99, 101))
        )])));
    }

    #[test]
    fn noncanonical_encodings_are_rejected() {
        for text in [
            r#"[{"protocol":"udp","ports":[{"start":1,"end":2}]},{"protocol":"tcp","ports":[{"start":1,"end":2}]}]"#,
            r#"[{"protocol":"tcp","ports":[{"start":1,"end":2},{"start":3,"end":4}]}]"#,
            r#"[{"protocol":"tcp","ports":[{"start":5,"end":4}]}]"#,
            r#"[{"protocol":"tcp","ports":[]}]"#,
            r#"[{"protocol":"icmpEcho","ports":[{"start":1,"end":1}]}]"#,
            r#"[{"protocol":"tcp","ports":[{"start":1,"end":1}],"extra":1}]"#,
        ] {
            assert!(serde_json::from_str::<ServiceSet>(text).is_err(), "{text}");
        }
        let full = ServiceSet::from_grants([(BusinessProtocol::Tcp, Some((1, u16::MAX)))]);
        let text = serde_json::to_string(&full).unwrap();
        assert_eq!(serde_json::from_str::<ServiceSet>(&text).unwrap(), full);
    }
}
