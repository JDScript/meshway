//! Staged migration only, deleted once every Node component applies Engine's
//! desired content: the one transitional issuer of component snapshots.
//! Control no longer issues. The daemon's Engine adapter binds each snapshot
//! to exactly the lease Engine accepted and withdraws it on Engine's empty
//! apply; test fixtures use the same issuer over Control's verified netmap.
use crate::{
    Clock, DataPlaneIdentity, DataPlaneKeys, Deadline, Error, ExecutionSnapshot, NetmapIssue,
    NetmapRenewal, Started,
};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::NetmapLease;
use meshway_protocol::InstanceRequest;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

pub struct StagedIssuer {
    clock: Arc<Clock>,
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    lineage: Arc<()>,
    issued: Option<(NetmapRenewal, DataPlaneIdentity)>,
}

impl StagedIssuer {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            clock: Arc::new(Clock::new()?),
            validity: Arc::new(AtomicBool::new(true)),
            invalidated: CancellationToken::new(),
            lineage: Arc::new(()),
            issued: None,
        })
    }

    /// The identity for `content` under `lease`. The same live content renews
    /// in place and rebinds the instance; new content retires every earlier
    /// snapshot first and continues the same lineage.
    pub fn issue(
        &mut self,
        content: &Arc<CompiledNetmap>,
        lease: NetmapLease,
        instance: Option<InstanceRequest>,
        keys: &DataPlaneKeys,
    ) -> Result<DataPlaneIdentity, Error> {
        let deadline = Deadline::from_response(Started::now(&self.clock)?, lease.expires_at_ms)?;
        if let Some((renewal, identity)) = &mut self.issued
            && renewal.reference() == content.reference()
            && identity.snapshot().ensure_current().is_ok()
        {
            if renewal.lease() != lease {
                renewal.renew(lease, deadline)?;
            }
            if identity.snapshot().instance() != instance {
                *identity = DataPlaneIdentity::new(renewal.snapshot(instance)?, keys)?;
            }
            return Ok(identity.clone());
        }
        self.retire();
        let (_, renewal) = ExecutionSnapshot::issue_netmap(NetmapIssue {
            netmap: content.clone(),
            lease,
            deadline,
            instance: None,
            validity: Arc::clone(&self.validity),
            invalidated: self.invalidated.clone(),
            clock: Arc::clone(&self.clock),
            lineage: Arc::downgrade(&self.lineage),
        })?;
        let identity = DataPlaneIdentity::new(renewal.snapshot(instance)?, keys)?;
        self.issued = Some((renewal, identity.clone()));
        Ok(identity)
    }

    /// The current identity, if one is issued and live.
    pub fn current(&self) -> Option<&DataPlaneIdentity> {
        self.issued
            .as_ref()
            .map(|(_, identity)| identity)
            .filter(|identity| identity.snapshot().ensure_current().is_ok())
    }

    fn retire(&mut self) {
        self.validity.store(false, Ordering::Release);
        self.invalidated.cancel();
        self.validity = Arc::new(AtomicBool::new(true));
        self.invalidated = CancellationToken::new();
        self.issued = None;
    }

    /// Engine's empty apply: every snapshot ends at once; the next content
    /// starts a new lineage and is never a continuation.
    pub fn withdraw(&mut self) {
        self.retire();
        self.lineage = Arc::new(());
    }
}
