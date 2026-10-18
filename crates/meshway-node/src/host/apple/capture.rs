//! Translate the Policy's disjoint ranges to NE routes, without re-evaluating rules.
use super::*;
use meshway_configuration::execution::EgressCapturePlan;

pub(super) const MAX_ROUTES: usize = 32_768;
pub(super) const JOURNAL_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn routes(plan: Option<&EgressCapturePlan>) -> Result<Vec<ipnet::IpNet>> {
    let Some(plan) = plan else {
        return Ok(Vec::new());
    };
    let routes: Vec<_> = plan
        .managed_ipv4
        .iter()
        .flat_map(|&(start, end)| ipnet::Ipv4Subnets::new(start, end, 0).map(ipnet::IpNet::V4))
        .chain(
            plan.managed_ipv6
                .iter()
                .chain(&plan.sink_ipv6)
                .flat_map(|&(start, end)| {
                    ipnet::Ipv6Subnets::new(start, end, 0).map(ipnet::IpNet::V6)
                }),
        )
        .take(MAX_ROUTES + 1)
        .collect();
    if routes.len() > MAX_ROUTES {
        return Err(Error::Unsupported("Apple capture prefix capacity"));
    }
    Ok(ipnet::IpNet::aggregate(&routes))
}
