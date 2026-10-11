use super::*;

impl Owner {
    pub(super) fn system_upstreams(
        &self,
        system: &crate::SystemResolverSnapshot,
        deadline: Instant,
    ) -> Result<VecDeque<SystemUpstream>, Unavailable> {
        use meshway_configuration::DnsTransport;
        if system.upstreams.is_empty() || system.upstreams.len() > 8 {
            return Err(Unavailable::MissingResolverPlan);
        }
        // The netmap decides each upstream from its egress partition.
        let route = |index, transport| {
            LocalResolverRoute::system(&self.desired, system.clone(), index, transport, deadline)
                .map_err(|error| match error {
                    crate::RouteError::Expired => Unavailable::Expired,
                    crate::RouteError::Binding => Unavailable::MissingResolverPlan,
                })
        };
        (0..system.upstreams.len())
            .map(|index| {
                Ok(SystemUpstream {
                    route: route(index, system.primary)?,
                    fallback: if system.primary == DnsTransport::Udp {
                        Some(route(index, DnsTransport::Tcp)?)
                    } else {
                        None
                    },
                })
            })
            .collect()
    }
}

impl Owner {
    /// A served source query at the resolver Node. Under `dns.overrideLocal`
    /// the configured default resolvers are its upstream, as for this Node's
    /// own ordinary names: the first one on its own netmap route. Otherwise
    /// every verified OS upstream in order, each with its own netmap-decided
    /// local route. The resolver's own split/MagicDNS selection does not
    /// apply to the source's query.
    pub(crate) fn begin_served(
        &mut self,
        query: Query,
        system: Option<&crate::SystemResolverSnapshot>,
        deadline: Instant,
    ) -> Begin {
        self.reap();
        if Plan::of(&self.desired) != Plan::ServeExitResolver {
            return Begin::Unavailable(Unavailable::MissingExitPlan);
        }
        let deadline = deadline.min(Instant::now() + self.state.limits.query_timeout);
        let dns = &self.desired.netmap().projection.configuration.dns;
        if dns.override_local {
            let Some(key) = dns.default.first() else {
                return Begin::Unavailable(Unavailable::MissingResolverPlan);
            };
            let mut route = match LocalResolverRoute::new(&self.desired, key, deadline) {
                Ok(route) => route,
                Err(crate::RouteError::Expired) => return Begin::Unavailable(Unavailable::Expired),
                Err(crate::RouteError::Binding) => {
                    return Begin::Unavailable(Unavailable::MissingResolverPlan);
                }
            };
            let ticket = match self.state.insert_pending(query.clone(), deadline) {
                Ok(ticket) => ticket,
                Err(reason) => return Begin::Unavailable(reason),
            };
            route.bind_cancellation(ticket.cancelled.clone());
            return Begin::Upstream(Box::new(UpstreamWork {
                ticket,
                route: ResolverRoute::Local(route),
                query,
                fallback: None,
                remaining: VecDeque::new(),
            }));
        }
        let Some(system) = system else {
            return Begin::Unavailable(Unavailable::MissingResolverPlan);
        };
        // A name the OS would route back to this Node's own DNS link would
        // be answered by this Node's own records and routes, not the OS.
        if system.routes_to_owner(&query.question().name().to_ascii()) {
            return Begin::Unavailable(Unavailable::MissingResolverPlan);
        }
        let mut upstreams = match self.system_upstreams(system, deadline) {
            Ok(upstreams) => upstreams,
            Err(reason) => return Begin::Unavailable(reason),
        };
        let ticket = match self.state.insert_pending(query.clone(), deadline) {
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
