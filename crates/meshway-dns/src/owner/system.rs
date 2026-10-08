use super::*;

impl Owner {
    pub(super) fn system_upstreams(
        &self,
        system: &crate::SystemResolverSnapshot,
        demands: &BTreeMap<usize, RequestId>,
        tcp_demands: &BTreeMap<usize, RequestId>,
        deadline: Instant,
    ) -> Result<VecDeque<SystemUpstream>, Unavailable> {
        use meshway_configuration::DnsTransport;
        if system.upstreams.is_empty() || system.upstreams.len() > 8 {
            return Err(Unavailable::MissingResolverPlan);
        }
        let route = |index, transport, ids: &BTreeMap<usize, RequestId>| {
            let demand = ids.get(&index).ok_or(Unavailable::MissingResolverPlan)?;
            LocalResolverRoute::system(
                self.snapshot.clone(),
                system.clone(),
                index,
                transport,
                *demand,
                deadline,
            )
            .map_err(|error| match error {
                crate::RouteError::Expired => Unavailable::Expired,
                crate::RouteError::Binding => Unavailable::MissingResolverPlan,
            })
        };
        (0..system.upstreams.len())
            .map(|index| {
                Ok(SystemUpstream {
                    route: route(index, system.primary, demands)?,
                    fallback: if system.primary == DnsTransport::Udp {
                        Some(route(index, DnsTransport::Tcp, tcp_demands)?)
                    } else {
                        None
                    },
                })
            })
            .collect()
    }
}
