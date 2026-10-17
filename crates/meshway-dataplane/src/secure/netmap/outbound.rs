use super::super::{CircuitGrant, Error, SecureNode, View};
use super::{Outbound, Preview};
use crate::{Packet, Purpose, netmap::Outcome};
use meshway_configuration::execution::{BusinessProtocol, LogicalPath, UnavailableReason};
use meshway_model::NodeId;
use meshway_policy::compile::netmap::Target;
use std::net::IpAddr;

/// A new local flow's route before any circuit: see `SecureNode::new_route`.
enum NewRoute {
    Paths(Target, Vec<LogicalPath>),
    Direct,
    Unmanaged,
    Denied(UnavailableReason),
    /// Permitted, but none of the decision's exits is ready for the
    /// destination's family.
    NoReadyExit(Target),
}

impl SecureNode {
    /// Admit a local TUN packet: an existing flow continues on its original
    /// path and circuit; a new flow is decided locally on the installed
    /// netmap. Nothing waits for a Server. Queued packets commit no flow;
    /// they are admitted again by [`Self::flush`].
    pub fn outbound(&mut self, packet: Packet) -> Result<Outbound, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.outbound");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        // A local packet originates here: it carries this Node's own address.
        let addresses = &desired.local().addresses;
        let own = match packet.facts().traffic.source {
            std::net::IpAddr::V4(address) => addresses.ipv4 == Some(address),
            std::net::IpAddr::V6(address) => addresses.ipv6 == Some(address),
        };
        if !own {
            return Err(Error::Packet);
        }
        self.maintain_circuits(&reading, false)?;
        let local = reading.node();
        if let Some(flow) = self.local_flow_at(&packet, &reading)? {
            let target = &flow.grant;
            let grant = CircuitGrant {
                purpose: Purpose::Business,
                target: target.clone(),
            };
            let live = flow.circuit.filter(|id| {
                self.circuits.get(id).is_some_and(|circuit| {
                    circuit.state.established() && circuit.check_at(&reading).is_ok()
                })
            });
            let established = live.or_else(|| {
                (flow.path.first() == Some(&local))
                    .then(|| self.established(&grant, &flow.path, &reading))
                    .flatten()
                    .map(|info| info.id)
            });
            if let Some(id) = established {
                if let Some(next) = self.missing_next_hop(id, local) {
                    // A live circuit whose next hop lost its adjacency: ask for
                    // that adjacency again and hold the packet, bounded, until
                    // the carrier is back or the circuit itself ends.
                    self.first.interest(next);
                    let path = LogicalPath {
                        nodes: flow.path.clone(),
                        dial_candidates: Default::default(),
                    };
                    let until = reading.instant + self.limits.handshake_timeout;
                    self.first.queue(target, &[path], packet, until);
                    return Ok(Outbound::AwaitingAdjacency);
                }
                return self
                    .prepare_send_at(id, packet, &reading)
                    .map(Outbound::Send);
            }
            if flow.path.first() != Some(&local) {
                return Err(Error::State);
            }
            // The origin reacquires a cipher only for the flow's unchanged path.
            let path = LogicalPath {
                nodes: flow.path.clone(),
                dial_candidates: Default::default(),
            };
            return self.wait(target.clone(), vec![path], packet, &reading);
        }
        let traffic = packet.facts().traffic;
        let (target, paths) = match self.new_route(
            &reading,
            traffic.source,
            traffic.destination,
            (traffic.protocol, traffic.destination_port),
        )? {
            NewRoute::Paths(target, paths) => (target, paths),
            NewRoute::Direct => return Ok(Outbound::Direct),
            NewRoute::Unmanaged => return Ok(Outbound::Unmanaged),
            NewRoute::Denied(reason) => return Ok(Outbound::Denied(reason)),
            NewRoute::NoReadyExit(target) => {
                // Fail closed, never direct: an IPv6 application learns it at
                // once (EACCES) and falls back to its IPv4 address.
                if let Some(error) = crate::packet::prohibited(&packet)
                    && let Some(cap) = crate::authority::Cap::after(
                        reading.instant,
                        reading.now_ms,
                        std::time::Duration::from_secs(1),
                    )
                    && let Ok(error) = crate::PendingRoute::refusal(
                        error,
                        target,
                        local,
                        cap,
                        self.cancellation.child_token(),
                    )
                {
                    self.mtu.report(error, reading.instant);
                }
                return Ok(Outbound::Denied(UnavailableReason::ExitUnavailable));
            }
        };
        let grant = CircuitGrant {
            purpose: Purpose::Business,
            target: target.clone(),
        };
        if let Some(info) = self.joinable(&grant, &paths, &reading) {
            return self
                .prepare_send_at(info.id, packet, &reading)
                .map(Outbound::Send);
        }
        self.wait(target, paths, packet, &reading)
    }

    /// A new local flow's decision on the desired netmap, with an exit
    /// decision narrowed to the paths of its first ready exit.
    fn new_route(
        &mut self,
        reading: &View<'_>,
        source: IpAddr,
        destination: IpAddr,
        service: (BusinessProtocol, Option<meshway_transport::spec::Port>),
    ) -> Result<NewRoute, Error> {
        let outcome = self
            .engine
            .decisions
            .decide(reading.netmap(), reading.node(), destination, service)
            .map_err(|_| Error::Policy)?;
        let decision = match outcome {
            Outcome::Permitted(decision) => decision,
            Outcome::Direct => return Ok(NewRoute::Direct),
            Outcome::Unmanaged => return Ok(NewRoute::Unmanaged),
            Outcome::Denied(reason) => return Ok(NewRoute::Denied(reason)),
        };
        let paths: Vec<LogicalPath> = match &decision.target {
            Target::Mesh { .. } => decision.paths.clone(),
            Target::Exit { nodes, .. } => {
                let family = meshway_model::exit::ExitFamily::of(destination);
                if family != meshway_model::exit::ExitFamily::of(source) {
                    return Ok(NewRoute::Denied(UnavailableReason::UnsupportedFamily));
                }
                let Some(exit) = self.first.choose_exit(nodes, family) else {
                    return Ok(NewRoute::NoReadyExit(decision.target.clone()));
                };
                decision
                    .paths
                    .iter()
                    .filter(|path| path.nodes.last() == Some(exit))
                    .cloned()
                    .collect()
            }
        };
        Ok(NewRoute::Paths(decision.target.clone(), paths))
    }

    /// What a new local flow toward `destination` does now: the decision and
    /// exit choice of [`Self::outbound`], and the complete path it takes:
    /// the established circuit it joins, else the path a new circuit takes
    /// ([`Self::choose_path`]), else the first candidate while no next hop
    /// has a carrier. Reporting only: nothing is admitted, opened or wanted.
    pub fn preview(
        &mut self,
        destination: IpAddr,
        service: (BusinessProtocol, Option<meshway_transport::spec::Port>),
    ) -> Result<Preview, Error> {
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        let addresses = &desired.local().addresses;
        let source = match destination {
            IpAddr::V4(_) => addresses.ipv4.map(IpAddr::V4),
            IpAddr::V6(_) => addresses.ipv6.map(IpAddr::V6),
        };
        let Some(source) = source.or(addresses.ipv4.map(IpAddr::V4)) else {
            return Ok(Preview::Denied(UnavailableReason::UnsupportedFamily));
        };
        Ok(
            match self.new_route(&reading, source, destination, service)? {
                NewRoute::Paths(target, paths) => {
                    let grant = CircuitGrant {
                        purpose: Purpose::Business,
                        target: target.clone(),
                    };
                    let (path, established) = self
                        .route_path(&grant, &paths, &reading)
                        .unwrap_or_default();
                    Preview::Mesh {
                        target,
                        path,
                        established,
                    }
                }
                NewRoute::Direct => Preview::Direct,
                NewRoute::Unmanaged => Preview::Unmanaged,
                NewRoute::Denied(reason) => Preview::Denied(reason),
                NewRoute::NoReadyExit(_) => Preview::Denied(UnavailableReason::ExitUnavailable),
            },
        )
    }

    /// The established business circuit toward `peer` a new flow joins now,
    /// in the order of [`Self::choose_path`]; reporting only.
    pub fn joined_route(&self, peer: NodeId) -> Option<Vec<NodeId>> {
        let target = Target::Mesh { node: peer };
        let paths = self
            .desired
            .compiler()
            .transit(self.desired.netmap().node_id, &target)
            .ok()?;
        let grant = CircuitGrant {
            purpose: Purpose::Business,
            target,
        };
        self.joinable(&grant, &paths, &View::of(&self.desired))
            .map(|info| info.path)
    }
}
