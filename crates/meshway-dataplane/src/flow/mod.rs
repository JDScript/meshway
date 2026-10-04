//! One bounded owner of forward facts. Preparation is read-only apart from
//! expired-entry collection; commit happens only after final guard validation.
mod tcp;

use crate::{Action, Direction, Error, Ingress, Limits, Packet, Route, Transport};
use meshway_configuration::execution::{BusinessProtocol, NodeConfiguration};
use meshway_model::{NetworkId, NodeId, PublicationId, RequestId};
use meshway_policy::admission::{Hop, Traffic, admit_forward, admit_reply};
use meshway_transport_spec::Port;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};
use tcp::TcpState;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Tuple {
    source: IpAddr,
    destination: IpAddr,
    protocol: u8,
    source_port: Option<Port>,
    destination_port: Option<Port>,
    echo: Option<(u16, u16)>,
}
impl Tuple {
    fn packet(packet: &Packet) -> Self {
        let traffic = packet.traffic();
        Self {
            source: traffic.source,
            destination: traffic.destination,
            protocol: match traffic.protocol {
                BusinessProtocol::Tcp => 6,
                BusinessProtocol::Udp => 17,
                BusinessProtocol::IcmpEcho => 1,
            },
            source_port: traffic.source_port,
            destination_port: traffic.destination_port,
            echo: match packet.transport() {
                Transport::Echo {
                    identifier,
                    sequence,
                    ..
                } => Some((identifier, sequence)),
                _ => None,
            },
        }
    }
    fn reversed(&self) -> Self {
        Self {
            source: self.destination,
            destination: self.source,
            source_port: self.destination_port,
            destination_port: self.source_port,
            ..self.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Origin,
    Relay,
    Terminal,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    network: NetworkId,
    publication: PublicationId,
    demand: RequestId,
    node: NodeId,
    role: Role,
    path: Vec<NodeId>,
    tuple: Tuple,
}

#[derive(Clone)]
enum ProtocolState {
    Udp,
    Echo,
    Tcp(TcpState),
}

#[derive(Clone)]
struct Flow {
    forward: Traffic,
    state: ProtocolState,
    expires_at: Instant,
    permit_expires_at_ms: u64,
    guard_expires_at_ms: u64,
}

pub(super) struct FlowTime {
    pub instant: Instant,
    pub now_ms: u64,
    pub guard_expires_at: Instant,
    pub guard_expires_at_ms: u64,
}

pub(super) struct Prepared {
    key: Key,
    flow: Flow,
    remove: bool,
    hop: Hop,
    direction: Direction,
    pub expires_at: Instant,
    pub permit_expires_at_ms: u64,
}

pub(super) struct FlowTable {
    limits: Limits,
    entries: BTreeMap<Key, Flow>,
}

impl FlowTable {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            entries: BTreeMap::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn expire(&mut self, now: Instant, now_ms: u64) {
        self.entries.retain(|_, flow| {
            now < flow.expires_at
                && now_ms < flow.permit_expires_at_ms
                && now_ms < flow.guard_expires_at_ms
        });
    }

    pub fn prepare(
        &mut self,
        config: &NodeConfiguration,
        ingress: &Ingress,
        packet: &Packet,
        time: FlowTime,
    ) -> Result<Prepared, Error> {
        self.expire(time.instant, time.now_ms);
        let path = ingress.path();
        // Match the compiler's bound before retaining any caller-owned path.
        if path.is_empty() || path.len() > meshway_policy::compile::MAX_HOPS {
            return Err(Error::Ingress);
        }
        let position = path
            .iter()
            .position(|node| *node == config.node_id)
            .ok_or(Error::Ingress)?;
        let origin = ingress.origin().ok_or(Error::Ingress)?;
        let tuple = Tuple::packet(packet);
        if tuple == tuple.reversed() {
            return Err(Error::Flow);
        }
        let mut key = Key {
            network: config.network_id,
            publication: config.publication,
            demand: ingress.demand(),
            node: config.node_id,
            role: if position == 0 {
                Role::Origin
            } else if position + 1 == path.len() {
                Role::Terminal
            } else {
                Role::Relay
            },
            path: path.to_vec(),
            tuple,
        };
        let reverse_key = Key {
            tuple: key.tuple.reversed(),
            ..key.clone()
        };
        // A second permitted candidate is not permission to move a live
        // session. Preserve the original path even when both paths are valid.
        if self.entries.keys().any(|other| {
            other.network == key.network
                && other.publication == key.publication
                && other.demand == key.demand
                && other.node == key.node
                && other.path != key.path
                && (other.tuple == key.tuple || other.tuple == reverse_key.tuple)
        }) {
            return Err(Error::Flow);
        }
        let (existing, direction) = if let Some(flow) = self.entries.get(&reverse_key) {
            key = reverse_key;
            (Some(flow), Direction::Reply)
        } else {
            (self.entries.get(&key), Direction::Forward)
        };
        let expected_sender = match direction {
            Direction::Forward => path.first(),
            Direction::Reply => path.last(),
        }
        .copied()
        .ok_or(Error::Ingress)?;
        if ingress.sender(config.node_id) != expected_sender {
            return Err(Error::Sender);
        }
        let hop = match direction {
            Direction::Forward => admit_forward(
                config,
                ingress.demand(),
                path,
                origin,
                ingress.previous(),
                &packet.traffic(),
                time.now_ms,
            )?,
            Direction::Reply => admit_reply(
                config,
                ingress.demand(),
                path,
                origin,
                ingress.previous(),
                &existing.ok_or(Error::Flow)?.forward,
                &packet.traffic(),
                time.now_ms,
            )?,
        };
        // Policy owns grant validity. Looking up its expiry here only bounds
        // retention and delayed output; it is not an alternate admission path.
        let permit_expires_at_ms = config
            .permits
            .get(&ingress.demand())
            .ok_or(Error::Ingress)?
            .expires_at_ms;
        let mut state = match existing {
            Some(flow) => flow.state.clone(),
            None => {
                if self.entries.len() == self.limits.max_flows {
                    return Err(Error::Capacity);
                }
                match packet.transport() {
                    Transport::Udp => ProtocolState::Udp,
                    Transport::Echo { reply: false, .. } => ProtocolState::Echo,
                    Transport::Tcp(segment) => ProtocolState::Tcp(TcpState::new(segment)?),
                    _ => return Err(Error::Flow),
                }
            }
        };
        let remove = if existing.is_some() {
            match (&mut state, packet.transport()) {
                (ProtocolState::Udp, Transport::Udp) => false,
                (ProtocolState::Echo, Transport::Echo { reply, .. })
                    if reply == (direction == Direction::Reply) =>
                {
                    false
                }
                (ProtocolState::Tcp(tcp), Transport::Tcp(segment)) => {
                    tcp.accept(direction, segment)?
                }
                _ => return Err(Error::Flow),
            }
        } else {
            false
        };
        let expires_at = time
            .instant
            .checked_add(self.limits.idle_timeout.min(Duration::from_millis(
                permit_expires_at_ms.saturating_sub(time.now_ms),
            )))
            .ok_or(Error::Limits)?
            .min(time.guard_expires_at);
        let flow = Flow {
            forward: existing.map_or_else(|| packet.traffic(), |flow| flow.forward),
            state,
            expires_at,
            permit_expires_at_ms,
            guard_expires_at_ms: time.guard_expires_at_ms,
        };
        Ok(Prepared {
            key,
            flow,
            remove,
            hop,
            direction,
            expires_at,
            permit_expires_at_ms,
        })
    }

    pub fn commit(&mut self, prepared: Prepared, packet: Packet) -> Action {
        let route = Route {
            demand: prepared.key.demand,
            origin: prepared.key.path[0],
            original_path: prepared.key.path.clone(),
            direction: prepared.direction,
        };
        if prepared.remove {
            self.entries.remove(&prepared.key);
        } else {
            self.entries.insert(prepared.key, prepared.flow);
        }
        match prepared.hop {
            Hop::Forward(next) => Action::Forward {
                next,
                packet,
                route,
            },
            Hop::Deliver => Action::Deliver { packet, route },
            Hop::Exit => Action::Exit { packet, route },
        }
    }
}

#[cfg(test)]
mod tests;
