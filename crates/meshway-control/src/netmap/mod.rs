//! The verified signed netmap and its lease. Control verifies, persists and
//! publishes it; Engine owns the only expiry timer and every component state.
//! Session and instance leases never bound it; stop/down, key rotation,
//! explicit identity rejection, an unknown private-write outcome and a newer
//! content replace it.
use crate::{Control, Error};
use meshway_configuration::netmap::CompiledNetmap;
use meshway_model::{NetmapLease, NetmapRef};
use meshway_protocol::{NetmapSignature, PeerFactsDelta, StateCursor};
use std::sync::Arc;
pub use sync::KnownPeers;

pub(crate) mod artifacts;
mod facts;
pub(crate) mod store;
mod sync;
pub use artifacts::{ArtifactCall, ArtifactProblem, ArtifactReply, fetch_artifacts};
pub use sync::{
    NetmapChange, PeerFactsSink, SYNC_TIMEOUT, SyncCall, SyncFuture, SyncOutcome, SyncReply,
    SyncTransport, sync_loop,
};

/// Same-content leases are written again once they extend the saved expiry
/// by this much, bounding writes to about one per hour of renewals.
const PERSIST_STEP_MS: u64 = 60 * 60 * 1000;

/// Private caches of the held identity, removed when the Node leaves.
pub(crate) const CACHE_FILE: &str = store::FILE;
pub(crate) const FACTS_FILE: &str = facts::FILE;

#[derive(Default)]
pub(crate) struct State {
    /// Verified content for this identity: the delta base, with or without a lease.
    base: Option<Arc<CompiledNetmap>>,
    /// Signed lease of `base` this identity may publish to Engine.
    lease: Option<NetmapLease>,
    persisted_expiry: Option<u64>,
    facts: Option<StateCursor>,
    /// Last known peer facts of `base`: they stay until replaced, removed
    /// or out of its projection, never by expiry. Saved in `peer-facts.json`.
    known: Option<facts::Known>,
    /// What `peer-facts.json` holds, as far as this owner knows.
    saved: Option<facts::Known>,
    /// `traversalRequests` of the last answer: hints, echoed in the next
    /// request and published with the facts; never saved.
    traversal: Arc<Vec<meshway_protocol::UdpTraversalRequest>>,
    /// `known` changed since it was last published to the facts sink.
    unpublished: bool,
    facts_write: Option<tokio::task::JoinHandle<Result<(), meshway_private_storage::Error>>>,
    /// Replaced whenever base, cursor or lease changes; a sync reply prepared
    /// against another value is superseded.
    exchange: Arc<()>,
    /// Rule-set artifacts `base` references.
    artifacts: artifacts::State,
    /// Verified DNS association overlay and its digest. Usable only while
    /// its netmap is `base`; never saved, so a restart starts without one.
    associations: Option<(
        meshway_model::Digest,
        Arc<meshway_configuration::netmap::DnsAssociationOverlay>,
    )>,
    /// This Node's routing selection as `base` decides it, computed once per content.
    routing: Option<meshway_configuration::selection::RoutingSelectionState>,
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
        let reference = netmap.reference();
        // The signed issue time is a trusted lower bound of the current time.
        self.clock.raise_floor(lease.issued_at_ms);
        let known = facts::load(&self.directory, &netmap.netmap, reference);
        self.hold(Arc::new(netmap));
        self.load_artifacts_blocking();
        if !authority || self.record.cache_blocked {
            return Ok(());
        }
        self.netmap.persisted_expiry = Some(lease.expires_at_ms);
        self.netmap.lease = Some(lease);
        self.netmap.unpublished = known.is_some();
        self.netmap.saved.clone_from(&known);
        self.netmap.known = known;
        Ok(())
    }

    /// `netmap` becomes the held content.
    fn hold(&mut self, netmap: Arc<CompiledNetmap>) {
        self.netmap.routing =
            meshway_policy::compile::netmap::routing_selection(&netmap.netmap).ok();
        self.netmap.base = Some(netmap);
    }

    /// This Node's routing profile selection as the held netmap decides it.
    pub fn routing_selection(
        &self,
    ) -> Option<&meshway_configuration::selection::RoutingSelectionState> {
        self.netmap.base.as_ref()?;
        self.netmap.routing.as_ref()
    }

    /// Control's last known facts of the held netmap, the single owner's
    /// value. The host publishes it where it publishes the held netmap
    /// without Server contact, so a restart dials offline.
    pub fn known_peers(&self) -> Option<KnownPeers> {
        let known = self.netmap.known.as_ref()?;
        let base = self.netmap.base.as_ref()?;
        (known.netmap == base.reference()).then(|| KnownPeers {
            netmap: known.netmap,
            peers: Arc::clone(&known.peers),
            traversal_requests: Arc::clone(&self.netmap.traversal),
        })
    }

    /// Publish [`Self::known_peers`] if it changed since the last publication.
    pub(crate) fn publish_facts(&mut self, sink: &mut (dyn PeerFactsSink + Send)) {
        if !std::mem::take(&mut self.netmap.unpublished) {
            return;
        }
        if let Some(known) = self.known_peers() {
            sink.publish(known);
        }
    }

    /// The traversal requests of an accepted answer replace the previous ones.
    pub(crate) fn record_traversal(&mut self, requests: meshway_protocol::UdpTraversalRequests) {
        let requests = requests.into_vec();
        if *self.netmap.traversal != requests {
            self.netmap.traversal = Arc::new(requests);
            self.netmap.unpublished = true;
        }
    }

    /// Fold one accepted delta into the last known facts of the held netmap.
    pub(crate) fn record_facts(&mut self, delta: &PeerFactsDelta) {
        let Some(reference) = self.netmap_reference() else {
            return;
        };
        let known = self.netmap.known.get_or_insert_with(|| facts::Known {
            netmap: reference,
            cursor: delta.cursor,
            peers: Default::default(),
        });
        known.cursor = delta.cursor;
        let changed = delta.apply_to(Arc::make_mut(&mut known.peers));
        self.netmap.unpublished |= !changed.is_empty();
        self.scope_facts();
    }

    /// New content: facts of Nodes outside its projection are dropped.
    fn scope_facts(&mut self) {
        if let (Some(known), Some(base)) = (&mut self.netmap.known, &self.netmap.base) {
            let before = known.clone();
            known.scope(&base.netmap, base.reference());
            self.netmap.unpublished |= *known != before;
        }
    }

    /// Save the last known facts when addresses, readiness or the netmap
    /// changed. Facts are not authority: a failed or interrupted write leaves
    /// the previous or the new complete file and fails nothing else.
    pub(crate) async fn persist_facts(&mut self) {
        let Some(known) = self.netmap.known.clone() else {
            return;
        };
        if self
            .netmap
            .saved
            .as_ref()
            .is_some_and(|saved| saved.same_content(&known))
        {
            return;
        }
        let Some(binding) = self.record.binding.as_ref() else {
            return;
        };
        let Ok(bytes) = facts::encode(&known, binding.network_id, binding.node_id) else {
            return;
        };
        // Joined before returning: an accepted write never outlives this
        // owner (a dropped Control releases its directory at once). Facts are
        // not authority, so a failed write only leaves the previous file.
        if let Some(previous) = self.netmap.facts_write.take() {
            let _ = previous.await;
        }
        let directory = Arc::clone(&self.directory);
        let write = self
            .netmap
            .facts_write
            .insert(tokio::task::spawn_blocking(move || {
                directory.replace(facts::FILE, &bytes)
            }));
        let result = write.await;
        self.netmap.facts_write = None;
        if matches!(result, Ok(Ok(()))) {
            self.netmap.saved = Some(known);
        }
    }

    /// Join the facts write still in flight (shutdown).
    pub(crate) async fn finish_facts_write(&mut self) {
        if let Some(write) = self.netmap.facts_write.take() {
            let _ = write.await;
        }
    }

    /// Withdraw the published lease. Verified content stays as delta base;
    /// only a new signed sync response publishes a lease again.
    pub(crate) fn revoke_netmap(&mut self) {
        self.netmap.lease = None;
        self.netmap.associations = None;
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
            artifacts: self.published_artifacts(),
            associations: self.held_associations().map(|(_, overlay)| overlay),
        })
    }

    /// The held overlay when it belongs to the held content.
    pub(crate) fn held_associations(
        &self,
    ) -> Option<(
        meshway_model::Digest,
        Arc<meshway_configuration::netmap::DnsAssociationOverlay>,
    )> {
        let base = self.netmap.base.as_ref()?.reference();
        self.netmap
            .associations
            .as_ref()
            .filter(|(_, overlay)| overlay.netmap == base)
            .cloned()
    }

    /// Replace the held overlay with a verified update for the held content,
    /// or drop one bound to other content.
    pub(crate) fn record_associations(
        &mut self,
        update: Option<meshway_protocol::DnsAssociationsUpdate>,
    ) -> Result<(), Error> {
        let Some(update) = update else {
            if self.held_associations().is_none() {
                self.netmap.associations = None;
            }
            return Ok(());
        };
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        let digest = store::verify_associations(binding, &update)?;
        if Some(update.overlay.netmap) != self.netmap_reference() {
            return Err(Error::Contract);
        }
        self.netmap.associations =
            (!update.overlay.associations.is_empty()).then(|| (digest, Arc::new(update.overlay)));
        Ok(())
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
            self.hold(Arc::clone(&netmap));
            self.scope_facts();
            // Published at once with what is cached; the rest is fetched.
            self.load_artifacts().await;
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

    /// Pin the endorsed successor of the Server key: `control.json` first,
    /// then `netmap.json` with the successor's signature of the installed
    /// lease, so the cache verifies under the new pin. From now on only the
    /// successor's signatures verify. A crash between the two writes leaves a
    /// cache that is not authority on restart (fail closed) until the next sync.
    async fn pin_server_key(
        &mut self,
        next: meshway_protocol::ServerSigningKey,
        lease: NetmapLease,
        signature: &NetmapSignature,
    ) -> Result<(), Error> {
        self.ready()?;
        let binding = self.record.binding.as_mut().ok_or(Error::Lifecycle)?;
        binding.server_signing_key = next;
        self.persist_binding().await?;
        let base = self.netmap.base.clone().ok_or(Error::Lifecycle)?;
        self.persist_netmap(&base, &lease, signature).await?;
        self.netmap.exchange = Arc::new(());
        Ok(())
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
