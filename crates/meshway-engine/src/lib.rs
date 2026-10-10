//! The Node's single desired-state owner.
//!
//! Control verifies and persists signed netmaps and owns their expiry timer.
//! The engine accepts content only: no component receives a lease, generation,
//! lineage, inherited scope or transferable output authority. Reconfiguration
//! is serialized with packet processing by the caller's exclusive engine borrow.
#![forbid(unsafe_code)]

use meshway_configuration::{
    execution::DialCandidate,
    netmap::{CompiledNetmap, EgressPlan, ExitRole, NetmapDns, NetmapSelf, NodeNetmap},
};
use meshway_model::NetmapRef;
use meshway_policy::compile::netmap::LocalCompiler;
use std::{collections::BTreeMap, sync::Arc};

/// Immutable desired inputs, built once for a content reference. Consumers
/// borrow the same projection/compiler; per-flow policy results stay lazy.
pub struct Desired {
    reference: NetmapRef,
    netmap: Arc<NodeNetmap>,
    compiler: Arc<LocalCompiler>,
}

impl Desired {
    fn new(netmap: &CompiledNetmap) -> Result<Self, Error> {
        netmap.verify().map_err(|_| Error::Content)?;
        let reference = netmap.reference();
        let netmap = Arc::new(netmap.netmap.clone());
        let compiler = Arc::new(LocalCompiler::from_shared(netmap.clone()));
        Ok(Self {
            reference,
            netmap,
            compiler,
        })
    }

    pub fn reference(&self) -> NetmapRef {
        self.reference
    }
    pub fn netmap(&self) -> &Arc<NodeNetmap> {
        &self.netmap
    }
    pub fn compiler(&self) -> &Arc<LocalCompiler> {
        &self.compiler
    }
    pub fn local(&self) -> &NetmapSelf {
        &self.netmap.self_
    }
    /// Candidate order/topology/keepalive are evaluated by the shared compiler
    /// on this projection, never inferred from observed peer availability.
    pub fn dialers(&self) -> &BTreeMap<String, DialCandidate> {
        &self.netmap.dialers
    }
    pub fn egress(&self) -> Option<&EgressPlan> {
        self.netmap.egress.as_ref()
    }
    pub fn exit_role(&self) -> Option<&ExitRole> {
        self.netmap.exit_role.as_ref()
    }
    pub fn dns(&self) -> &NetmapDns {
        &self.netmap.dns
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
}

impl<C: Components> Engine<C> {
    pub fn new(components: C) -> Self {
        Self {
            components,
            desired: None,
        }
    }
    pub fn desired(&self) -> Option<&Desired> {
        self.desired.as_ref()
    }

    /// Sole content transition. Same reference is a no-op: lease renewal is
    /// entirely Control's timer responsibility and never calls components.
    pub async fn reconfig(&mut self, netmap: Option<&CompiledNetmap>) -> Result<(), Error> {
        if let (Some(current), Some(next)) = (&self.desired, netmap) {
            let reference = next.reference();
            if current.reference == reference {
                return Ok(());
            }
            if current.reference.serial >= reference.serial {
                return Err(Error::Reference);
            }
        }
        let desired = netmap.map(Desired::new).transpose()?;
        if let Err(error) = self.apply(desired.as_ref()).await {
            // Cleanup always attempts every owner, even when one failed. No
            // partially installed desired state becomes the current engine.
            self.desired = None;
            let _ = self.apply(None).await;
            return Err(error);
        }
        self.desired = desired;
        Ok(())
    }

    async fn apply(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        // Remove inadmissible packet state before changing physical resources.
        // Packet processing cannot interleave with this exclusive borrow.
        let packets = self.components.apply_packets(desired).await;
        if desired.is_some() {
            packets?;
        }
        let adjacencies = self.components.apply_adjacencies(desired).await;
        if desired.is_some() {
            adjacencies?;
        }
        let host = self.components.apply_host(desired).await;
        if desired.is_some() {
            host?;
        }
        let dns = self.components.apply_dns(desired).await;
        packets.and(adjacencies).and(host).and(dns)
    }
}
