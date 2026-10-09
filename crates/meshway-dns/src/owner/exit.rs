use super::*;
use crate::MeshResolverRoute;
use meshway_configuration::DnsTransport;

impl Owner {
    pub(super) fn begin_exit(&mut self, query: Query, request_deadline: Instant) -> Begin {
        let Some(plan) = Plan::of(&self.snapshot) else {
            return Begin::Unavailable(Unavailable::Inactive);
        };
        let Plan::ViaExit {
            max_pending,
            timeout_ms,
        } = plan
        else {
            return Begin::Unavailable(Unavailable::MissingExitPlan);
        };
        if self.state.pending.len() >= usize::from(max_pending) {
            return Begin::Unavailable(Unavailable::Capacity);
        }
        let deadline = request_deadline.min(self.snapshot.expires_at()).min(
            Instant::now()
                + self
                    .state
                    .limits
                    .query_timeout
                    .min(Duration::from_millis(u64::from(timeout_ms))),
        );
        let routes = (|| {
            Ok::<_, crate::RouteError>((
                MeshResolverRoute::new(self.snapshot.clone(), DnsTransport::Udp, deadline)?,
                MeshResolverRoute::new(self.snapshot.clone(), DnsTransport::Tcp, deadline)?,
            ))
        })();
        let (mut route, mut tcp) = match routes {
            Ok(routes) => routes,
            Err(crate::RouteError::Expired) => return Begin::Unavailable(Unavailable::Expired),
            Err(_) => return Begin::Unavailable(Unavailable::MissingExitPlan),
        };
        let ticket = match self.state.insert_pending(query.clone(), deadline) {
            Ok(ticket) => ticket,
            Err(reason) => return Begin::Unavailable(reason),
        };
        route.bind_cancellation(ticket.cancelled.clone());
        tcp.bind_cancellation(ticket.cancelled.clone());
        Begin::Upstream(Box::new(UpstreamWork {
            ticket,
            query,
            route: ResolverRoute::Mesh(route),
            fallback: Some(ResolverRoute::Mesh(tcp)),
            // Failure at the selected exit can never try the client's OS list.
            remaining: VecDeque::new(),
        }))
    }
}
