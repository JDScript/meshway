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
            // A netmap decides each upstream from its egress partition.
            let demand = ids.get(&index).copied();
            if demand.is_none() && !self.snapshot.is_netmap() {
                return Err(Unavailable::MissingResolverPlan);
            }
            LocalResolverRoute::system(
                self.snapshot.clone(),
                system.clone(),
                index,
                transport,
                demand,
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

impl Owner {
    /// A served source query at the resolver Node: every verified OS upstream
    /// in order, each with its own netmap-decided local route. The resolver's
    /// own split/MagicDNS selection does not apply to the source's query.
    pub(crate) fn begin_served(
        &mut self,
        query: Query,
        system: &crate::SystemResolverSnapshot,
        deadline: Instant,
    ) -> Begin {
        self.reap();
        if !self.snapshot.is_netmap() || Plan::of(&self.snapshot) != Some(Plan::ServeExitResolver) {
            return Begin::Unavailable(Unavailable::MissingExitPlan);
        }
        let deadline = deadline
            .min(self.snapshot.expires_at())
            .min(Instant::now() + self.state.limits.query_timeout);
        let none = BTreeMap::new();
        let mut upstreams = match self.system_upstreams(system, &none, &none, deadline) {
            Ok(upstreams) => upstreams,
            Err(reason) => return Begin::Unavailable(reason),
        };
        let ticket = match self.state.insert_pending(
            query.clone(),
            String::new(),
            PendingKind::Upstream,
            deadline,
        ) {
            Ok(ticket) => ticket,
            Err(reason) => return Begin::Unavailable(reason),
        };
        for upstream in &mut upstreams {
            upstream.route.bind_cancellation(ticket.cancelled.clone());
            if let Some(fallback) = &mut upstream.fallback {
                fallback.bind_cancellation(ticket.cancelled.clone());
            }
        }
        let first = upstreams.pop_front().expect("nonempty checked system list");
        Begin::Upstream(Box::new(UpstreamWork {
            ticket,
            route: ResolverRoute::Local(first.route),
            query,
            fallback: first.fallback.map(ResolverRoute::Local),
            remaining: upstreams,
        }))
    }
}
