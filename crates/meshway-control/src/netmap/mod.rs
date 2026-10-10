//! Netmap execution authority: the only state from which Control issues
//! netmap snapshots. Session and instance leases never bound it; only the
//! netmap lease, stop/down, key rotation, explicit identity rejection, a newer
//! content and an unknown private-write outcome end it.
use crate::{Control, Error, Started};
use meshway_authority::{Deadline, ExecutionSnapshot, NetmapIssue, NetmapRenewal};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{NetmapLease, NetmapRef};
use meshway_protocol::{NetmapSignature, StateCursor};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

pub(crate) mod store;
mod sync;
pub use sync::{
    FactsRejected, NetmapChange, PeerFactsSink, SYNC_TIMEOUT, SyncCall, SyncFuture, SyncOutcome,
    SyncReply, SyncTransport, sync_loop,
};

/// Same-content leases are written again once they extend the saved expiry
/// by this much, bounding writes to about one per hour of renewals.
const PERSIST_STEP_MS: u64 = 60 * 60 * 1000;

pub(crate) struct State {
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    lineage: Arc<()>,
    /// Verified content for this identity: the delta base, with or without authority.
    base: Option<Arc<CompiledNetmap>>,
    installed: Option<NetmapRenewal>,
    persisted_expiry: Option<u64>,
    facts: Option<StateCursor>,
    /// Replaced whenever base, cursor or authority changes; a sync reply
    /// started under another epoch is superseded.
    epoch: Arc<()>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            validity: Arc::new(AtomicBool::new(true)),
            invalidated: CancellationToken::new(),
            lineage: Arc::new(()),
            base: None,
            installed: None,
            persisted_expiry: None,
            facts: None,
            epoch: Arc::new(()),
        }
    }
}

impl State {
    fn cancel_guards(&mut self) {
        self.validity.store(false, Ordering::Release);
        self.invalidated.cancel();
        self.validity = Arc::new(AtomicBool::new(true));
        self.invalidated = CancellationToken::new();
        self.installed = None;
        self.epoch = Arc::new(());
    }

    /// Cancels every current netmap guard if dropped before `disarm`.
    pub(crate) fn fuse(&self) -> Fuse {
        Fuse {
            validity: Arc::clone(&self.validity),
            invalidated: self.invalidated.clone(),
            armed: true,
        }
    }

    pub(crate) fn shutdown(&self) {
        self.validity.store(false, Ordering::Release);
        self.invalidated.cancel();
    }

    fn issue(
        &self,
        clock: &Arc<meshway_authority::Clock>,
        netmap: Arc<CompiledNetmap>,
        lease: NetmapLease,
        deadline: Deadline,
    ) -> Result<NetmapRenewal, Error> {
        let (_, renewal) = ExecutionSnapshot::issue_netmap(NetmapIssue {
            netmap,
            lease,
            deadline,
            instance: None,
            validity: Arc::clone(&self.validity),
            invalidated: self.invalidated.clone(),
            clock: Arc::clone(clock),
            lineage: Arc::downgrade(&self.lineage),
        })?;
        Ok(renewal)
    }
}

pub(crate) struct Fuse {
    validity: Arc<AtomicBool>,
    invalidated: CancellationToken,
    armed: bool,
}

impl Fuse {
    pub(crate) fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for Fuse {
    fn drop(&mut self) {
        if self.armed {
            self.validity.store(false, Ordering::Release);
            self.invalidated.cancel();
        }
    }
}

impl Control {
    /// Restart path: a verified cache of this identity becomes the delta base;
    /// when it carries a valid pinned-Server signature and its lease is still
    /// live it is execution authority again without any Server contact.
    pub(crate) fn load_netmap(&mut self) -> Result<(), Error> {
        let Some(binding) = self.record.binding.as_ref() else {
            return Ok(());
        };
        let Some(store::Loaded {
            netmap,
            lease,
            authority,
        }) = store::load(&self.directory, binding)?
        else {
            return Ok(());
        };
        let netmap = Arc::new(netmap);
        self.netmap.base = Some(Arc::clone(&netmap));
        if !authority || self.record.cache_blocked {
            return Ok(());
        }
        self.netmap.persisted_expiry = Some(lease.expires_at_ms);
        if let Ok(deadline) =
            Deadline::from_response(Started::now(&self.clock)?, lease.expires_at_ms)
        {
            self.netmap.installed = self.netmap.issue(&self.clock, netmap, lease, deadline).ok();
        }
        Ok(())
    }

    /// Stop every current netmap guard and start a new lineage. Verified
    /// content stays as delta base; only a new sync response re-issues authority.
    pub(crate) fn revoke_netmap(&mut self) {
        self.netmap.cancel_guards();
        self.netmap.lineage = Arc::new(());
    }

    /// Current netmap snapshot, bound to the current instance when one is live.
    /// Neither an absent session nor an expired instance ends it.
    pub fn netmap_snapshot(&self) -> Result<ExecutionSnapshot, Error> {
        self.ready()?;
        let renewal = self.netmap.installed.as_ref().ok_or(Error::Lifecycle)?;
        Ok(renewal.snapshot(self.current_instance().ok())?)
    }

    /// Verified content held, with or without authority; comparison data only.
    pub(crate) fn netmap_base(&self) -> Option<&Arc<CompiledNetmap>> {
        self.netmap.base.as_ref()
    }

    /// Verified content held (the next sync base), with or without authority.
    pub fn netmap_reference(&self) -> Option<NetmapRef> {
        self.netmap.base.as_ref().map(|netmap| netmap.reference())
    }

    /// Current lease of the installed netmap, if any authority is installed.
    pub fn netmap_lease(&self) -> Option<NetmapLease> {
        self.netmap
            .installed
            .as_ref()
            .map(NetmapRenewal::lease)
            .filter(|_| self.netmap_snapshot().is_ok())
    }

    /// Install verified content and its lease. Same content renews the lease
    /// in place when the installed authority is live, otherwise re-issues it.
    /// New content must have a larger serial; it cancels old guards (a new
    /// lineage if the own identity changed), is saved, and only then issued.
    async fn install_netmap(
        &mut self,
        netmap: Arc<CompiledNetmap>,
        lease: NetmapLease,
        signature: &NetmapSignature,
        deadline: Deadline,
    ) -> Result<NetmapChange, Error> {
        let reference = netmap.reference();
        let previous = self.netmap_reference();
        if previous.is_some_and(|old| old != reference && reference.serial <= old.serial) {
            return Err(Error::Contract);
        }
        if self.record.cache_blocked {
            self.record.cache_blocked = false;
            self.persist().await?;
        }
        let change = if previous == Some(reference) {
            let base = self.netmap.base.clone().ok_or(Error::Lifecycle)?;
            let renewed = self
                .netmap
                .installed
                .as_ref()
                .is_some_and(|renewal| renewal.renew(lease, deadline.clone()).is_ok());
            if renewed {
                NetmapChange::Renewed
            } else {
                self.netmap.cancel_guards();
                self.netmap.installed =
                    Some(self.netmap.issue(&self.clock, base, lease, deadline)?);
                NetmapChange::Reissued
            }
        } else {
            if let Some(base) = &self.netmap.base {
                let (old, new) = (&base.netmap, &netmap.netmap);
                if old.self_.identity != new.self_.identity
                    || old.self_.addresses != new.self_.addresses
                    || old.self_.self_keys != new.self_.self_keys
                {
                    self.revoke_netmap();
                }
            }
            self.netmap.cancel_guards();
            self.persist_netmap(&netmap, &lease, signature).await?;
            self.netmap.base = Some(Arc::clone(&netmap));
            self.netmap.installed =
                Some(self.netmap.issue(&self.clock, netmap, lease, deadline)?);
            NetmapChange::New { previous }
        };
        if !matches!(change, NetmapChange::New { .. })
            && self
                .netmap
                .persisted_expiry
                .is_none_or(|saved| lease.expires_at_ms >= saved.saturating_add(PERSIST_STEP_MS))
        {
            let base = self.netmap.base.clone().ok_or(Error::Lifecycle)?;
            self.persist_netmap(&base, &lease, signature).await?;
        }
        self.netmap.epoch = Arc::new(());
        Ok(change)
    }

    /// Atomic replacement of `netmap.json`. Either the old or the new sealed
    /// file survives a crash, and both are valid caches, so an unknown outcome
    /// does not require reopening: it fails this installation only.
    async fn persist_netmap(
        &mut self,
        netmap: &CompiledNetmap,
        lease: &NetmapLease,
        signature: &NetmapSignature,
    ) -> Result<(), Error> {
        self.ready()?;
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        let keys = self.record.keys.as_ref().ok_or(Error::Record)?;
        let bytes = store::encode(netmap, lease, signature, binding, keys)?;
        if let Some(previous) = self.netmap_write.take() {
            let _ = previous.await;
        }
        let directory = Arc::clone(&self.directory);
        let write = self
            .netmap_write
            .insert(tokio::task::spawn_blocking(move || {
                crate::storage::ensure_marker(&directory)
                    .map_err(|_| meshway_private_storage::Error::UnsafeStorage)?;
                directory.replace(store::FILE, &bytes)
            }));
        let result = write.await.map_err(|_| Error::ReloadRequired)?;
        self.netmap_write = None;
        result?;
        self.netmap.persisted_expiry = Some(lease.expires_at_ms);
        Ok(())
    }
}
