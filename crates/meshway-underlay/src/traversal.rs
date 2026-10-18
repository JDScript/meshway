//! Target authorization paired with one protected descriptor. No socket alias or
//! arbitrary recipient-adder escapes this module.
use crate::{Error, Result};
use meshway_control::connectivity::{
    AttemptScope, DiscoveryContext, SocketReference, udp_recipient,
};
use meshway_transport::SendLifetime;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// Sources one attempt may learn besides its offer recipient.
const MAX_LEARNED: usize = 2;

/// Platform-owned descriptor lifetime and resolver. This adapter cannot grant
/// traversal recipients: the common authorization below performs every check.
pub trait Context: Send + Sync + 'static {
    fn execution(&self) -> Result<meshway_engine::Desired>;
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<SocketAddr>>>;
}

#[derive(Clone)]
pub struct DatagramAuthorization {
    registry: Arc<Registry>,
    context: Arc<dyn Context>,
    binding: Arc<Mutex<Binding>>,
    local: SocketAddr,
}
struct Binding {
    reference: SocketReference,
}

pub struct Registry {
    pub closed: CancellationToken,
    targets: Mutex<Vec<Target>>,
    channels: Mutex<std::collections::BTreeMap<SocketAddr, ChannelTarget>>,
}
struct ChannelTarget {
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
    pub fn new(
        context: Arc<dyn Context>,
        reference: SocketReference,
        local: SocketAddr,
    ) -> (Self, Arc<Registry>) {
        let registry = Arc::new(Registry {
            closed: CancellationToken::new(),
            targets: Mutex::new(Vec::new()),
            channels: Mutex::new(std::collections::BTreeMap::new()),
        });
        (
            Self {
                registry: registry.clone(),
                binding: Arc::new(Mutex::new(Binding { reference })),
                context,
                local,
            },
            registry,
        )
    }
    /// The descriptor is live (its purpose is unchanged on the installed
    /// content) and still names this Node. Its State reference is rebound by
    /// the Manager's apply; the Server checks it on every coordination step.
    fn check(&self) -> Result<SocketReference> {
        let desired = self.context.execution()?;
        let binding = self.binding.lock().map_err(|_| Error::Worker)?;
        if self.registry.closed.is_cancelled()
            || binding.reference.node_id != desired.netmap().node_id
        {
            return Err(Error::Expired);
        }
        Ok(binding.reference.clone())
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
        if context.netmap() != self.check()?.netmap {
            return Err(Error::Expired);
        }
        let server = context
            .servers()
            .iter()
            .find(|server| server.id == server_id)
            .ok_or(Error::Target)?;
        let addresses = tokio::select! { biased;
            _ = context.ended() => return Err(Error::Expired),
            value = self.context.resolve(&server.endpoint.host, server.endpoint.port.get()) => value?,
        };
        self.check()?;
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
        self.insert(self.attempt_peer(scope)?, Lifetime::Attempt(scope.clone()))
    }

    /// The offer's peer recipient, once the attempt is live on this socket.
    fn attempt_peer(&self, scope: &AttemptScope) -> Result<SocketAddr> {
        scope.ensure_current().map_err(|_| Error::Expired)?;
        let offer = scope.validated_offer();
        self.check()?;
        if offer.local().candidate.socket
            != self.binding.lock().map_err(|_| Error::Worker)?.reference
        {
            return Err(Error::Target);
        }
        udp_recipient(&offer.peer().address).map_err(|_| Error::Target)
    }

    /// Addresses this attempt reaches besides its offer recipient.
    fn learned(targets: &[Target], scope: &AttemptScope, peer: SocketAddr) -> Vec<SocketAddr> {
        targets
            .iter()
            .filter_map(|target| match &target.lifetime {
                Lifetime::Attempt(other)
                    if other.validated_offer().offer() == scope.validated_offer().offer()
                        && target.address != peer =>
                {
                    Some(target.address)
                }
                _ => None,
            })
            .collect()
    }

    /// A source the endpoint's authentication owner verified a signed message
    /// of this attempt from: the peer's NAT translated the offer's address to
    /// another port or public address. Any valid unicast recipient, at most
    /// [`MAX_LEARNED`] per attempt, and only for the attempt's lifetime.
    pub fn authorize_learned(&self, scope: &AttemptScope, address: SocketAddr) -> Result<()> {
        let peer = self.attempt_peer(scope)?;
        let address = udp_recipient(&address.to_string()).map_err(|_| Error::Target)?;
        if address == peer {
            return Ok(());
        }
        {
            let mut targets = self.registry.targets.lock().map_err(|_| Error::Worker)?;
            targets.retain(|target| target.lifetime.current());
            let learned = Self::learned(&targets, scope, peer);
            if learned.contains(&address) {
                return Ok(());
            }
            if learned.len() >= MAX_LEARNED {
                return Err(Error::Capacity);
            }
        }
        self.insert(address, Lifetime::Attempt(scope.clone()))
    }

    /// The endpoint's authentication owner transfers a live attempt target to
    /// its selected channel guard. Revoking the attempt cannot extend this guard.
    /// `address` is the offer recipient or a source learned for this attempt.
    pub fn authorize_channel(
        &self,
        scope: &AttemptScope,
        lifetime: Arc<dyn SendLifetime>,
        address: SocketAddr,
    ) -> Result<()> {
        let peer = self.attempt_peer(scope)?;
        if address != peer
            && !Self::learned(
                &self.registry.targets.lock().map_err(|_| Error::Worker)?,
                scope,
                peer,
            )
            .contains(&address)
        {
            return Err(Error::Target);
        }
        if !lifetime.is_current() {
            return Err(Error::Expired);
        }
        let mut channels = self.registry.channels.lock().map_err(|_| Error::Worker)?;
        channels.retain(|_, channel| channel.lifetime.is_current());
        if channels.len() >= 64 {
            return Err(Error::Capacity);
        }
        channels.insert(address, ChannelTarget { lifetime });
        Ok(())
    }

    /// New content for this descriptor's Host-installed target. The same
    /// physical socket identity carries the new State reference; discovery
    /// and attempt targets of earlier content end, and only selected channels
    /// whose own lifetime is still live keep their recipient.
    pub fn rebind(&self, reference: &SocketReference) -> Result<()> {
        let desired = self.context.execution()?;
        if self.registry.closed.is_cancelled() {
            return Err(Error::Expired);
        }
        let mut binding = self.binding.lock().map_err(|_| Error::Worker)?;
        if reference.node_id != binding.reference.node_id
            || reference.socket_id != binding.reference.socket_id
            || reference.socket_generation != binding.reference.socket_generation
            || reference.resource != binding.reference.resource
            || !references(&desired, reference)?
        {
            return Err(Error::Target);
        }
        self.registry
            .channels
            .lock()
            .map_err(|_| Error::Worker)?
            .retain(|_, channel| channel.lifetime.is_current());
        self.registry
            .targets
            .lock()
            .map_err(|_| Error::Worker)?
            .clear();
        binding.reference = reference.clone();
        Ok(())
    }
}

/// The reference names this content's Node and netmap. Its instance is State
/// qualification only and may be rebound while the netmap stays the same.
pub fn references(desired: &meshway_engine::Desired, reference: &SocketReference) -> Result<bool> {
    Ok(reference.node_id == desired.netmap().node_id && reference.netmap == desired.reference())
}
