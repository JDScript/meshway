//! The Node's single desired-state owner.
//!
//! Control verifies and persists signed netmaps. Engine owns their only expiry
//! timer. Components accept content only: none receives a lease, generation,
//! lineage, inherited scope or transferable output authority. Reconfiguration
//! is serialized with packet processing by the caller's exclusive engine borrow.
#![forbid(unsafe_code)]

use meshway_configuration::{
    execution::DialCandidate,
    netmap::{CompiledNetmap, EgressPlan, ExitRole, NetmapDns, NetmapSelf, NodeNetmap},
};
use meshway_model::{NetmapLease, NetmapRef};
use meshway_policy::compile::netmap::LocalCompiler;
use std::{collections::BTreeMap, sync::Arc};

mod lease;
pub use lease::Netmap;
#[cfg(test)]
mod tests;

/// Immutable desired inputs, built once for a content reference. Consumers
/// borrow the same projection/compiler; per-flow policy results stay lazy.
/// Cloning shares both.
#[derive(Clone)]
pub struct Desired {
    content: Arc<CompiledNetmap>,
    compiler: Arc<LocalCompiler>,
}

impl Desired {
    /// Content plus its one shared compiler. Holding a `Desired` grants
    /// nothing: components act only on what Engine applies to them.
    pub fn new(content: &Arc<CompiledNetmap>) -> Result<Self, Error> {
        content.verify().map_err(|_| Error::Content)?;
        let compiler = Arc::new(LocalCompiler::from_shared(content.netmap.clone()));
        Ok(Self {
            content: content.clone(),
            compiler,
        })
    }

    pub fn reference(&self) -> NetmapRef {
        self.content.reference()
    }
    /// Verified content, shared with Control's cache.
    pub fn content(&self) -> &Arc<CompiledNetmap> {
        &self.content
    }
    pub fn netmap(&self) -> &Arc<NodeNetmap> {
        &self.content.netmap
    }
    pub fn compiler(&self) -> &Arc<LocalCompiler> {
        &self.compiler
    }
    pub fn local(&self) -> &NetmapSelf {
        &self.content.netmap.self_
    }
    /// Candidate order/topology/keepalive are evaluated by the shared compiler
    /// on this projection, never inferred from observed peer availability.
    pub fn dialers(&self) -> &BTreeMap<String, DialCandidate> {
        &self.content.netmap.dialers
    }
    pub fn egress(&self) -> Option<&EgressPlan> {
        self.content.netmap.egress.as_ref()
    }
    pub fn exit_role(&self) -> Option<&ExitRole> {
        self.content.netmap.exit_role.as_ref()
    }
    pub fn dns(&self) -> &NetmapDns {
        &self.content.netmap.dns
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid netmap content")]
    Content,
    #[error("netmap reference regressed or was reused for different content")]
    Reference,
    #[error("required Node component could not apply desired state")]
    Apply,
    #[error("netmap lease is invalid or has expired")]
    Lease,
}

/// Production adapters own mechanisms, not authority. Each operation is
/// idempotent and diffs its actual installed resources against `desired`.
/// Before returning, it joins retired work so that old buffered output cannot
/// escape after reconfig. `None` admits no traffic; host cleanup retains the
/// negative capture boundary until explicit operator down removes ownership.
#[allow(async_fn_in_trait)]
pub trait Components {
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error>;
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error>;
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error>;
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error>;
}

pub struct Engine<C> {
    components: C,
    desired: Option<Desired>,
    lease: Option<lease::Active>,
    latest: Option<NetmapLease>,
    clock: lease::Clock,
}

impl<C: Components> Engine<C> {
    pub fn new(components: C) -> Self {
        Self {
            components,
            desired: None,
            lease: None,
            latest: None,
            clock: lease::Clock::new(),
        }
    }
    pub fn desired(&self) -> Option<&Desired> {
        self.desired.as_ref()
    }

    /// Mechanisms between reconfigurations: packet, adjacency and DNS work
    /// borrow the components exclusively, so it cannot interleave with apply.
    pub fn components(&self) -> &C {
        &self.components
    }
    pub fn components_mut(&mut self) -> &mut C {
        &mut self.components
    }
    pub fn into_components(self) -> C {
        self.components
    }

    /// Sole execution transition. Control has already verified the signature
    /// and identity binding. Renewing live content changes only this owner's
    /// timer; an expired engine must first apply empty, then install anew.
    pub async fn reconfig(&mut self, netmap: Option<&Netmap>) -> Result<(), Error> {
        let active = netmap
            .map(|next| lease::Active::accept(next, self.latest, &mut self.clock))
            .transpose()?;
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.expired(&mut self.clock))
        {
            self.withdraw().await?;
        }
        if let (Some(current), Some(next)) = (&self.desired, netmap) {
            let reference = next.content.reference();
            if current.reference() == reference {
                self.latest = Some(next.lease);
                self.lease = active;
                return Ok(());
            }
            if current.reference().serial >= reference.serial {
                return Err(Error::Reference);
            }
        }
        let desired = netmap.map(|next| Desired::new(&next.content)).transpose()?;
        // Removing the current desired state before the first await is the
        // engine's admission boundary. If its caller cancels this operation,
        // the engine stays empty; the next reconfig retries idempotent applies.
        self.desired = None;
        self.lease = None;
        let result = match &active {
            Some(lease) => tokio::time::timeout_at(lease.at, self.apply(desired.as_ref()))
                .await
                .unwrap_or(Err(Error::Lease)),
            None => self.apply(None).await,
        };
        if let Err(error) = result {
            // Cleanup always attempts every owner, even when one failed. No
            // partially installed desired state becomes the current engine.
            let _ = self.apply(None).await;
            return Err(error);
        }
        if let Some(next) = netmap {
            self.latest = Some(next.lease);
        }
        self.lease = active;
        self.desired = desired;
        Ok(())
    }

    /// Selected by the engine's event loop, independently of Control HTTP and
    /// component work. This is scheduling data; components never receive it.
    pub fn next_expiry(&self) -> Option<tokio::time::Instant> {
        self.lease.as_ref().map(|lease| lease.at)
    }

    /// Consume the timer event. The owner also calls this on a wall-clock
    /// change; a backward clock cannot extend its previously accepted lease.
    pub async fn expire(&mut self) -> Result<(), Error> {
        if self
            .lease
            .as_ref()
            .is_some_and(|lease| lease.expired(&mut self.clock))
        {
            self.withdraw().await?;
        }
        Ok(())
    }

    async fn withdraw(&mut self) -> Result<(), Error> {
        self.desired = None;
        self.lease = None;
        self.apply(None).await
    }

    async fn apply(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        if desired.is_some() {
            // Packet processing cannot interleave with this exclusive borrow,
            // so physical owners come first: capture/TUN, then adjacency, then
            // the flows/circuits narrowed to the new content, then DNS answers.
            self.components.apply_host(desired).await?;
            self.components.apply_adjacencies(desired).await?;
            self.components.apply_packets(desired).await?;
            return self.components.apply_dns(desired).await;
        }
        // Withdrawal: inadmissible packet state and queued output end before
        // physical resources change. Every owner is attempted.
        let packets = self.components.apply_packets(None).await;
        let adjacencies = self.components.apply_adjacencies(None).await;
        let host = self.components.apply_host(None).await;
        let dns = self.components.apply_dns(None).await;
        packets.and(adjacencies).and(host).and(dns)
    }
}
