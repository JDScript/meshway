use crate::{Error, Result};
use ipnet::{IpNet, Ipv4Net};
use meshway_configuration::execution::EgressCapturePlan;
use meshway_policy::compile::{MAX_CAPTURE_RANGES, MAX_CAPTURE_ROUTES};
use meshway_rules::ranges;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

/// Elements of a staged gate: the union of the old and the new plan.
pub(in crate::linux::kernel) const UNION_RANGES: usize = 2 * MAX_CAPTURE_RANGES;

/// Inclusive, ordered, disjoint and non-adjacent ranges: the elements of the
/// capture interval sets, exactly as `meshway-policy` computed them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ranges {
    pub v4: Vec<(Ipv4Addr, Ipv4Addr)>,
    pub v6: Vec<(Ipv6Addr, Ipv6Addr)>,
}

impl Ranges {
    /// The ranges of whole prefixes, such as the Mesh pools.
    pub fn of(nets: impl IntoIterator<Item = IpNet>) -> Self {
        let (mut v4, mut v6) = (Vec::new(), Vec::new());
        for net in nets {
            match net {
                IpNet::V4(net) => v4.push((net.network(), net.broadcast())),
                IpNet::V6(net) => v6.push((net.network(), net.broadcast())),
            }
        }
        Self {
            v4: ranges::merge(v4),
            v6: ranges::merge(v6),
        }
    }

    pub fn canonical(&self) -> bool {
        ranges::is_merged(&self.v4) && ranges::is_merged(&self.v6)
    }

    /// Routes of the capture table: the minimal CIDR cover of the IPv4 ranges.
    pub fn routes(&self) -> impl Iterator<Item = Ipv4Net> + '_ {
        self.v4
            .iter()
            .flat_map(|(start, end)| ranges::cover(*start, *end))
            .filter_map(|(network, bits)| Ipv4Net::new(network, bits).ok())
    }

    pub fn within_limits(&self) -> bool {
        self.v4.len() + self.v6.len() <= MAX_CAPTURE_RANGES
            && self.routes().nth(MAX_CAPTURE_ROUTES).is_none()
    }
}

pub(super) fn parse(plan: &EgressCapturePlan) -> Result<Ranges> {
    if plan.unavailable.is_some() {
        return Ok(Ranges {
            v4: vec![(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST)],
            v6: vec![(Ipv6Addr::UNSPECIFIED, Ipv6Addr::from(u128::MAX))],
        });
    }
    let ranges = Ranges {
        v4: plan.managed_ipv4.clone(),
        v6: plan.sink_ipv6.clone(),
    };
    if !ranges.canonical() || (plan.default_capture && ranges.v4.is_empty()) {
        return Err(Error::Target);
    }
    if !ranges.within_limits() {
        return Err(Error::Capacity);
    }
    Ok(ranges)
}

pub(super) fn union(a: &Ranges, b: &Ranges) -> Ranges {
    Ranges {
        v4: ranges::merge(a.v4.iter().chain(&b.v4).copied().collect()),
        v6: ranges::merge(a.v6.iter().chain(&b.v6).copied().collect()),
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn ranges_parse_union_and_cover() {
        let a: Ipv4Addr = "10.0.0.0".parse().unwrap();
        let b: Ipv4Addr = "10.0.0.255".parse().unwrap();
        let c: Ipv4Addr = "10.0.1.0".parse().unwrap();
        let d: Ipv4Addr = "10.0.1.7".parse().unwrap();
        let x = Ranges {
            v4: vec![(a, b)],
            v6: vec![],
        };
        let y = Ranges {
            v4: vec![(c, d)],
            v6: vec![],
        };
        assert!(x.canonical() && y.canonical());
        let both = union(&x, &y);
        assert_eq!(both.v4, [(a, d)]);
        assert_eq!(
            both.routes().map(|n| n.to_string()).collect::<Vec<_>>(),
            ["10.0.0.0/24", "10.0.1.0/29"]
        );
        let touching = Ranges {
            v4: vec![(a, b), (c, d)],
            v6: vec![],
        };
        assert!(!touching.canonical());
        let mesh = Ranges::of([
            "100.64.0.0/10".parse().unwrap(),
            "fd00::/8".parse().unwrap(),
        ]);
        assert_eq!(mesh.v4.len(), 1);
        assert_eq!(mesh.v6.len(), 1);
    }
}
