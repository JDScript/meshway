use crate::{Error, Result};
use ipnet::IpNet;
use meshway_configuration::execution::EgressCapturePlan;
use serde::{Deserialize, Serialize};

pub(super) const LIMIT: usize = 4096;
pub(in crate::linux::kernel) const RESERVED: u32 = 20_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Ranges {
    pub v4: Vec<IpNet>,
    pub v6: Vec<IpNet>,
}

pub(super) fn parse(plan: &EgressCapturePlan) -> Result<Ranges> {
    let groups = [
        &plan.managed_ipv4,
        &plan.direct_ipv4,
        &plan.unmanaged_ipv4,
        &plan.sink_ipv6,
    ];
    if groups.iter().map(|group| group.len()).sum::<usize>() > LIMIT {
        return Err(Error::Capacity);
    }
    let mut all = Vec::<IpNet>::new();
    let mut result = Ranges::default();
    for (group, values) in groups.iter().enumerate() {
        for value in values.iter() {
            let net = value.parse::<IpNet>().map_err(|_| Error::Target)?;
            if net.addr() != net.network()
                || net.addr().is_ipv6() != (group == 3)
                || all.iter().any(|old| overlaps(*old, net))
            {
                return Err(Error::Target);
            }
            all.push(net);
            match group {
                0 => result.v4.push(net),
                3 => result.v6.push(net),
                _ => {}
            }
        }
    }
    if plan.default_capture && result.v4.is_empty() {
        return Err(Error::Target);
    }
    if plan.unavailable.is_some() {
        result.v4 = vec!["0.0.0.0/0".parse().expect("constant")];
        result.v6 = vec!["::/0".parse().expect("constant")];
    }
    Ok(result)
}
fn overlaps(a: IpNet, b: IpNet) -> bool {
    a.contains(&b.network()) || b.contains(&a.network())
}
pub(super) fn union(a: &Ranges, b: &Ranges) -> Ranges {
    fn merged(a: &[IpNet], b: &[IpNet]) -> Vec<IpNet> {
        let mut values: Vec<_> = a.iter().chain(b).copied().collect();
        values.sort_by_key(|net| (net.prefix_len(), net.network()));
        let mut out: Vec<IpNet> = Vec::new();
        for value in values {
            if !out.iter().any(|net| net.contains(&value.network())) {
                out.push(value);
            }
        }
        out
    }
    Ranges {
        v4: merged(&a.v4, &b.v4),
        v6: merged(&a.v6, &b.v6),
    }
}
