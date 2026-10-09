//! Target authorization paired with one protected descriptor. No socket alias or
//! arbitrary recipient-adder escapes this module.
use super::{ProtectedDatagram, dns::Resolver, protection::ProtectionScope};
use crate::{Error, Result, target::View};
use meshway_control::connectivity::{
    AttemptScope, DiscoveryContext, SocketReference, udp_recipient,
};
use meshway_transport::SendLifetime;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

pub struct TraversalDatagram {
    pub endpoint: ProtectedDatagram,
    pub authorization: DatagramAuthorization,
}

#[derive(Clone)]
pub struct DatagramAuthorization {
    registry: Arc<Registry>,
    scope: ProtectionScope,
    resolver: Resolver,
    binding: Arc<Mutex<Binding>>,
    local: SocketAddr,
}
struct Binding {
    reference: SocketReference,
    snapshot: meshway_authority::ExecutionSnapshot,
}

pub(super) struct Registry {
    pub closed: CancellationToken,
    targets: Mutex<Vec<Target>>,
    channels: Mutex<std::collections::BTreeMap<SocketAddr, ChannelTarget>>,
}
struct ChannelTarget {
    peer: meshway_model::NodeId,
    carrier: meshway_configuration::execution::Carrier,
    lifetime: Arc<dyn SendLifetime>,
}
struct Target {
    address: SocketAddr,
    lifetime: Lifetime,
}
enum Lifetime {
    Discovery(DiscoveryContext),
    Attempt(AttemptScope),
}
impl Lifetime {
    fn current(&self) -> bool {
        match self {
            Self::Discovery(value) => value.ensure_current().is_ok(),
            Self::Attempt(value) => value.ensure_current().is_ok(),
        }
    }
}
impl Registry {
    pub fn allows(&self, address: SocketAddr) -> bool {
        !self.closed.is_cancelled()
            && (self.channels.lock().is_ok_and(|channels| {
                channels
                    .get(&address)
                    .is_some_and(|channel| channel.lifetime.is_current())
            }) || self.targets.lock().is_ok_and(|mut targets| {
                targets.retain(|target| target.lifetime.current());
                targets.iter().any(|target| target.address == address)
            }))
    }
}

impl DatagramAuthorization {
    pub(super) fn attach(
        mut endpoint: ProtectedDatagram,
        resolver: Resolver,
        reference: SocketReference,
    ) -> Result<TraversalDatagram> {
        let local = endpoint.local_addr()?;
        let scope = endpoint.scope();
        let registry = Arc::new(Registry {
            closed: CancellationToken::new(),
            targets: Mutex::new(Vec::new()),
            channels: Mutex::new(std::collections::BTreeMap::new()),
        });
        endpoint.traversal = Some(registry.clone());
        Ok(TraversalDatagram {
            endpoint,
            authorization: Self {
                registry,
                binding: Arc::new(Mutex::new(Binding {
                    reference,
                    snapshot: scope.execution()?,
                })),
                scope,
                resolver,
                local,
            },
        })
    }
    fn check(&self, snapshot: &meshway_authority::ExecutionSnapshot) -> Result<()> {
        self.scope.owner_current()?;
        let binding = self.binding.lock().map_err(|_| Error::Worker)?;
        binding
            .snapshot
            .ensure_current()
            .map_err(|_| Error::Expired)?;
        if self.registry.closed.is_cancelled() || !references(snapshot, &binding.reference)? {
            return Err(Error::Expired);
        }
        self.scope.shared.data(snapshot)?;
        Ok(())
    }
    fn insert(&self, address: SocketAddr, lifetime: Lifetime) -> Result<()> {
        if !lifetime.current() || address.is_ipv4() != self.local.is_ipv4() {
            return Err(Error::Target);
        }
        let mut targets = self.registry.targets.lock().map_err(|_| Error::Worker)?;
        targets.retain(|target| target.lifetime.current());
        if targets.len() >= 64 {
            return Err(Error::Capacity);
        }
        targets.push(Target { address, lifetime });
        Ok(())
    }
    pub async fn authorize_stun(
        &self,
        context: &DiscoveryContext,
        server_id: &str,
    ) -> Result<Vec<SocketAddr>> {
        context.ensure_current().map_err(|_| Error::Expired)?;
        self.check(context.snapshot())?;
        let server = context
            .servers()
            .iter()
            .find(|server| server.id == server_id)
            .ok_or(Error::Target)?;
        let resolver = self
            .resolver
            .scoped(self.scope.shared.data(context.snapshot())?)?;
        let addresses = tokio::select! { biased;
            _ = context.invalidated() => return Err(Error::Expired),
            value = resolver.resolve(&server.endpoint.host, server.endpoint.port.get()) => value?,
        };
        self.check(context.snapshot())?;
        let mut targets = Vec::new();
        for address in addresses {
            if address.is_ipv4() == self.local.is_ipv4()
                && udp_recipient(&address.to_string()).is_ok()
                && !targets.contains(&address)
            {
                self.insert(address, Lifetime::Discovery(context.clone()))?;
                targets.push(address);
                if targets.len() == 2 {
                    break;
                }
            }
        }
        Ok(targets)
    }
    pub fn authorize_attempt(&self, scope: &AttemptScope) -> Result<()> {
        scope.ensure_current().map_err(|_| Error::Expired)?;
        let offer = scope.validated_offer();
        self.check(offer.snapshot())?;
        if offer.local().candidate.socket
            != self.binding.lock().map_err(|_| Error::Worker)?.reference
        {
            return Err(Error::Target);
        }
        self.insert(
            udp_recipient(&offer.peer().address).map_err(|_| Error::Target)?,
            Lifetime::Attempt(scope.clone()),
        )
    }
    /// The endpoint's authentication owner transfers a live attempt target to
    /// its selected channel guard. Revoking the attempt cannot extend this guard.
    pub fn authorize_channel(
        &self,
        scope: &AttemptScope,
        lifetime: Arc<dyn SendLifetime>,
    ) -> Result<()> {
        scope.ensure_current().map_err(|_| Error::Expired)?;
        let offer = scope.validated_offer();
        self.check(offer.snapshot())?;
        if offer.local().candidate.socket
            != self.binding.lock().map_err(|_| Error::Worker)?.reference
        {
            return Err(Error::Target);
        }
        let address = udp_recipient(&offer.peer().address).map_err(|_| Error::Target)?;
        if !lifetime.is_current() {
            return Err(Error::Expired);
        }
        let mut channels = self.registry.channels.lock().map_err(|_| Error::Worker)?;
        channels.retain(|_, channel| channel.lifetime.is_current());
        if channels.len() >= 64 {
            return Err(Error::Capacity);
        }
        channels.insert(
            address,
            ChannelTarget {
                peer: offer.peer().candidate.socket.node_id,
                carrier: offer.offer().carrier.clone(),
                lifetime,
            },
        );
        Ok(())
    }

    /// Transfer only previously authenticated targets on this descriptor. The
    /// caller is the selected-channel owner and supplies new frozen peer guards.
    pub fn handoff(
        &self,
        snapshot: &meshway_authority::ExecutionSnapshot,
        reference: &SocketReference,
        selected: &[(SocketAddr, Arc<dyn SendLifetime>)],
    ) -> Result<()> {
        self.scope.owner_current()?;
        self.scope.shared.data(snapshot)?;
        if self.registry.closed.is_cancelled() {
            return Err(Error::Expired);
        }
        let mut binding = self.binding.lock().map_err(|_| Error::Worker)?;
        let (previous, next) = continued(snapshot, &binding.snapshot)?;
        if !crate::resource::compatible(previous, next, &binding.reference.resource)
            || reference.node_id != binding.reference.node_id
            || reference.instance != binding.reference.instance
            || reference.socket_id != binding.reference.socket_id
            || reference.socket_generation != binding.reference.socket_generation
            || reference.resource != binding.reference.resource
            || !references(snapshot, reference)?
        {
            return Err(Error::Target);
        }
        let mut channels = self.registry.channels.lock().map_err(|_| Error::Worker)?;
        if selected.len() > 64
            || selected.iter().any(|(address, scope)| {
                channels.get(address).is_none_or(|channel| {
                    previous.peer(channel.peer) != next.peer(channel.peer)
                        || admit_adjacency(snapshot, channel.peer, &channel.carrier).is_err()
                }) || !scope.is_current()
            })
        {
            return Err(Error::Target);
        }
        continued(snapshot, &binding.snapshot)?;
        channels.retain(|address, _| selected.iter().any(|(selected, _)| selected == address));
        for (address, scope) in selected {
            channels.get_mut(address).ok_or(Error::Target)?.lifetime = scope.clone();
        }
        self.registry
            .targets
            .lock()
            .map_err(|_| Error::Worker)?
            .clear();
        binding.reference = reference.clone();
        binding.snapshot = snapshot.clone();
        Ok(())
    }
}

/// The reference names this snapshot's Node and netmap. Its instance is State
/// qualification only and may be rebound while the netmap stays the same.
pub(super) fn references(
    snapshot: &meshway_authority::ExecutionSnapshot,
    reference: &SocketReference,
) -> Result<bool> {
    let netmap = snapshot.netmap().map_err(|_| Error::Expired)?;
    Ok(reference.node_id == netmap.netmap.node_id && reference.netmap == netmap.reference())
}

/// Current permission for a retained physical adjacency.
fn admit_adjacency(
    snapshot: &meshway_authority::ExecutionSnapshot,
    peer: meshway_model::NodeId,
    carrier: &meshway_configuration::execution::Carrier,
) -> Result<()> {
    let reading = snapshot.netmap_reading().map_err(|_| Error::Expired)?;
    let netmap = &reading.netmap.netmap;
    let compiler = meshway_policy::compile::netmap::LocalCompiler::new(netmap);
    meshway_policy::admission::netmap::admit_adjacency(&compiler, netmap, peer, carrier)
        .map_err(|_| Error::Expired)
}

/// Control's continuation from the descriptor's previous netmap to `next`,
/// as views of both contents. Comparison data only.
fn continued<'a>(
    next: &'a meshway_authority::ExecutionSnapshot,
    previous: &'a meshway_authority::ExecutionSnapshot,
) -> Result<(View<'a>, View<'a>)> {
    let new = next.netmap().map_err(|_| Error::Expired)?;
    if next.same_netmap(previous) {
        return Ok((View::new(&new.netmap), View::new(&new.netmap)));
    }
    let old = next
        .netmap_continuation_from(previous)
        .map_err(|_| Error::Expired)?;
    Ok((View::new(&old.netmap), View::new(&new.netmap)))
}
