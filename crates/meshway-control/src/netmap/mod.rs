//! The verified signed netmap and its lease. Control verifies, persists and
//! publishes it; Engine owns the only expiry timer and every component state.
//! Session and instance leases never bound it; stop/down, key rotation,
//! explicit identity rejection, an unknown private-write outcome and a newer
//! content replace it.
use crate::{Control, Error};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{NetmapLease, NetmapRef};
use meshway_protocol::{NetmapSignature, StateCursor};
use std::sync::Arc;

pub(crate) mod store;
mod sync;
pub use sync::{
    FactsRejected, NetmapChange, PeerFactsSink, SYNC_TIMEOUT, SyncCall, SyncFuture, SyncOutcome,
    SyncReply, SyncTransport, sync_loop,
};

/// Same-content leases are written again once they extend the saved expiry
/// by this much, bounding writes to about one per hour of renewals.
const PERSIST_STEP_MS: u64 = 60 * 60 * 1000;

#[derive(Default)]
pub(crate) struct State {
    /// Verified content for this identity: the delta base, with or without a lease.
    base: Option<Arc<CompiledNetmap>>,
    /// Signed lease of `base` this identity may publish to Engine.
    lease: Option<NetmapLease>,
    persisted_expiry: Option<u64>,
    facts: Option<StateCursor>,
    /// Replaced whenever base, cursor or lease changes; a sync reply prepared
    /// against another value is superseded.
    exchange: Arc<()>,
}

impl Control {
    /// Restart path: a verified cache of this identity becomes the delta base;
    /// with a valid pinned-Server signature its lease is published again
    /// without any Server contact. Engine rejects it if it has expired.
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
        self.netmap.base = Some(Arc::new(netmap));
        if !authority || self.record.cache_blocked {
            return Ok(());
        }
        self.netmap.persisted_expiry = Some(lease.expires_at_ms);
        self.netmap.lease = Some(lease);
        Ok(())
    }

    /// Withdraw the published lease. Verified content stays as delta base;
    /// only a new signed sync response publishes a lease again.
    pub(crate) fn revoke_netmap(&mut self) {
        self.netmap.lease = None;
        self.netmap.exchange = Arc::new(());
    }

    /// The verified netmap and its signed lease for Engine, unless that lease
    /// has already ended on Control's clock. Engine owns the expiry timer;
    /// Control never publishes an ended lease again.
    pub fn netmap(&self) -> Option<meshway_engine::Netmap> {
        self.ready().ok()?;
        let lease = self.netmap.lease?;
        if !lease.valid_at(self.clock.now().ok()?) {
            return None;
        }
        Some(meshway_engine::Netmap {
            content: self.netmap.base.clone()?,
            lease,
        })
    }

    /// Verified content held, with or without a lease; comparison data only.
    pub(crate) fn netmap_base(&self) -> Option<&Arc<CompiledNetmap>> {
        self.netmap.base.as_ref()
    }

    /// Verified content held (the next sync base), with or without a lease.
    pub fn netmap_reference(&self) -> Option<NetmapRef> {
        self.netmap.base.as_ref().map(|netmap| netmap.reference())
    }

    /// Published lease, while it has not ended on Control's clock.
    pub fn netmap_lease(&self) -> Option<NetmapLease> {
        self.netmap().map(|netmap| netmap.lease)
    }

    /// The held content when it is exactly `reference` and its lease is live:
    /// State, receipts, coordination and observations speak for it only then.
    pub(crate) fn held_netmap(&self, reference: NetmapRef) -> Result<Arc<CompiledNetmap>, Error> {
        self.netmap_lease()
            .filter(|lease| lease.netmap == reference)
            .ok_or(Error::Expired)?;
        self.netmap.base.clone().ok_or(Error::Expired)
    }

    /// Install verified content and its lease. Same content replaces the lease
    /// unless it would end earlier than the published one: a replayed signed
    /// lease cannot extend anything, and a Server clock stepping back (an
    /// earlier `issuedAt`) is not a replay. New content must have a larger serial;
    /// it is saved before it is published.
    async fn install_netmap(
        &mut self,
        netmap: Arc<CompiledNetmap>,
        lease: NetmapLease,
        signature: &NetmapSignature,
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
            if self
                .netmap
                .lease
                .is_some_and(|old| lease.expires_at_ms < old.expires_at_ms)
            {
                return Err(Error::Contract);
            }
            NetmapChange::Renewed
        } else {
            self.persist_netmap(&netmap, &lease, signature).await?;
            self.netmap.base = Some(Arc::clone(&netmap));
            NetmapChange::New { previous }
        };
        self.netmap.lease = Some(lease);
        if !matches!(change, NetmapChange::New { .. })
            && self
                .netmap
                .persisted_expiry
                .is_none_or(|saved| lease.expires_at_ms >= saved.saturating_add(PERSIST_STEP_MS))
        {
            let base = self.netmap.base.clone().ok_or(Error::Lifecycle)?;
            self.persist_netmap(&base, &lease, signature).await?;
        }
        self.netmap.exchange = Arc::new(());
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
