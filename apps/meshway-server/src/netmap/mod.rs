//! Netmap distribution: the long-poll `POST /agent-api/v1/sync`.
//!
//! A sync request names the netmap and peer-fact cursor the Node holds. The
//! Server answers at once when either is behind; otherwise it holds the
//! request until a committed change wakes it, the peer-fact source reports a
//! change or a traversal attempt the Node does not hold yet, a newer request
//! of the same Node replaces it, or the hold times out. A hold never outlasts
//! the lease the Node holds less [`SYNC_RENEWAL_MARGIN`]. Every answer carries a fresh lease for the current netmap. Content is
//! stored by `store::netmap` and only changes in stable-input transactions.
pub(crate) mod associations;
mod hub;
pub(crate) mod records;
pub mod signing;
mod sync;

pub(crate) use hub::Hub;
pub use sync::{NoPeerFacts, PeerFactsChanged, PeerFactsRead, PeerFactsSource};

use crate::{error::Error, execution::proof, secrets::Sealer, store::Handle};
use meshway_model::InstanceId;
use meshway_protocol::netmap::{SyncRequest, SyncResponse};
use std::sync::Arc;
use std::time::Duration;

/// Default lease validity of every sync response.
pub const DEFAULT_NETMAP_VALIDITY: Duration = Duration::from_secs(24 * 60 * 60);
/// Default nominal hold of a sync request without changes.
pub const DEFAULT_SYNC_HOLD: Duration = Duration::from_secs(50);
/// Largest configurable nominal hold; with +10% jitter a hold stays below 60 s.
pub const MAX_SYNC_HOLD: Duration = Duration::from_secs(54);
/// A held sync answers at least this long before the lease the Node holds
/// ends: the Node's largest transport backoff (15 s), so even a failed
/// answer leaves it one retry before it fails closed.
pub const SYNC_RENEWAL_MARGIN: Duration = Duration::from_secs(15);

/// The hold of one request: the nominal hold, cut so the answer leaves the
/// Node [`SYNC_RENEWAL_MARGIN`] of the lease it holds. A Node whose held
/// lease this Server does not know (its first request to this process) is
/// answered at once.
fn hold_within_lease(hold: Duration, held_until_ms: Option<u64>, now_ms: u64) -> Duration {
    let Some(until) = held_until_ms else {
        return Duration::ZERO;
    };
    hold.min(
        Duration::from_millis(until.saturating_sub(now_ms)).saturating_sub(SYNC_RENEWAL_MARGIN),
    )
}

/// Lease validity and hold timeout, validated when the App opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SyncTiming {
    pub validity: Duration,
    pub hold: Duration,
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    #[test]
    fn configured_validity_has_an_availability_floor_and_authority_ceiling() {
        for (seconds, valid) in [
            (59, false),
            (60, true),
            (300, true),
            (86400, true),
            (86401, false),
        ] {
            assert_eq!(
                SyncTiming {
                    validity: Duration::from_secs(seconds),
                    ..SyncTiming::default()
                }
                .valid(),
                valid
            );
        }
    }

    #[test]
    fn a_hold_leaves_the_node_a_renewal_before_its_lease_ends() {
        let hold = Duration::from_secs(55);
        // A 60 s lease with 50 s left: answered 15 s before it ends.
        assert_eq!(
            hold_within_lease(hold, Some(60_000), 10_000),
            Duration::from_secs(35)
        );
        assert_eq!(hold_within_lease(hold, Some(86_400_000), 0), hold);
        assert_eq!(
            hold_within_lease(hold, Some(20_000), 10_000),
            Duration::ZERO
        );
        assert_eq!(hold_within_lease(hold, Some(5_000), 10_000), Duration::ZERO);
        assert_eq!(hold_within_lease(hold, None, 0), Duration::ZERO);
    }
}

impl Default for SyncTiming {
    fn default() -> Self {
        Self {
            validity: DEFAULT_NETMAP_VALIDITY,
            hold: DEFAULT_SYNC_HOLD,
        }
    }
}

impl SyncTiming {
    pub(crate) fn valid(&self) -> bool {
        let validity = u64::try_from(self.validity.as_millis()).unwrap_or(u64::MAX);
        (meshway_model::netmap::MIN_NETMAP_VALIDITY_MS
            ..=meshway_model::netmap::MAX_NETMAP_VALIDITY_MS)
            .contains(&validity)
            && self.hold >= Duration::from_secs(1)
            && self.hold <= MAX_SYNC_HOLD
    }
    /// The nominal hold with uniform ±10% jitter, so Nodes that started
    /// together do not keep returning together.
    fn jittered_hold(&self) -> Duration {
        let mut sample = [0u8; 2];
        let fraction = match getrandom::fill(&mut sample) {
            Ok(()) => f64::from(u16::from_le_bytes(sample)) / f64::from(u16::MAX),
            Err(_) => 0.5,
        };
        self.hold.mul_f64(0.9 + 0.2 * fraction)
    }
}

/// Node-facing netmap distribution service.
#[derive(Clone)]
pub struct Netmaps {
    handle: Handle,
    sealer: Arc<Sealer>,
    boot: InstanceId,
    hub: Arc<Hub>,
    facts: Arc<dyn PeerFactsSource>,
    signer: Arc<signing::NetmapSigningKeys>,
    timing: SyncTiming,
    associations: associations::Shared,
}

impl Netmaps {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        boot: InstanceId,
        hub: Arc<Hub>,
        facts: Arc<dyn PeerFactsSource>,
        signer: Arc<signing::NetmapSigningKeys>,
        timing: SyncTiming,
        associations: associations::Shared,
    ) -> Self {
        Self {
            handle,
            sealer,
            boot,
            hub,
            facts,
            signer,
            timing,
            associations,
        }
    }

    /// Complete the standing association task of the calling exit for one
    /// source query (see `associations`), then wake both Nodes' syncs.
    pub async fn report_association(
        &self,
        token: &str,
        report: meshway_protocol::DnsAssociationReport,
    ) -> Result<meshway_protocol::DnsAssociationAccepted, Error> {
        let token = proof::token_hash(token)?;
        let (sealer, boot, store) = (self.sealer.clone(), self.boot, self.associations.clone());
        let (network, accepted, woken) = self
            .handle
            .read(move |tx| {
                let now = crate::store::execution::now()?;
                let principal =
                    crate::store::execution::authorize_node(tx, &sealer, boot, token, now)?;
                let now_ms =
                    u64::try_from(now).map_err(|_| Error::Internal("system clock before epoch"))?;
                let (accepted, woken) = store.complete(
                    tx,
                    &sealer,
                    principal.network,
                    principal.node,
                    &report,
                    now_ms,
                )?;
                Ok((principal.network, accepted, woken))
            })
            .await?;
        self.hub.wake(network, &woken);
        Ok(accepted)
    }

    /// Holds the request until something the Node lacks exists, then answers
    /// with the current lease; see the module documentation.
    pub async fn sync(&self, token: &str, request: SyncRequest) -> Result<SyncResponse, Error> {
        let token = proof::token_hash(token)?;
        let principal = {
            let (sealer, boot) = (self.sealer.clone(), self.boot);
            self.handle
                .read(move |tx| {
                    crate::store::execution::authorize_node(
                        tx,
                        &sealer,
                        boot,
                        token,
                        crate::store::execution::now()?,
                    )
                })
                .await?
        };
        records::Records::shared().received(
            principal.network,
            principal.node,
            chrono::Utc::now().timestamp_millis().max(0) as u64,
            request.server_key_generation,
        );
        let answered = |waiter: &hub::Waiter, response: SyncResponse| {
            records::Records::shared().issued(principal.network, principal.node, response.lease);
            waiter.answered(response.lease.expires_at_ms);
            response
        };
        // Register before the first read: a change committed after this point
        // wakes the waiter, one committed before it is visible to the read.
        let mut waiter = self.hub.register(principal.network, principal.node)?;
        let deadline = tokio::time::Instant::now()
            + hold_within_lease(
                self.timing.jittered_hold(),
                waiter.held_until_ms(),
                chrono::Utc::now().timestamp_millis().max(0) as u64,
            );
        loop {
            let (response, changed) = self.read(token, &request).await?;
            if !matches!(
                response.netmap,
                meshway_protocol::netmap::SyncNetmap::Unchanged
            ) || response
                .peer_facts
                .as_ref()
                .is_some_and(|facts| facts.base.is_none() || Some(facts.cursor) != request.state)
                || response.associations.is_some()
                || response.traversal_requests.as_slice().iter().any(|asked| {
                    !request
                        .traversal_requests
                        .as_slice()
                        .contains(&asked.attempt_id)
                })
            {
                return Ok(answered(&waiter, response));
            }
            let facts_changed = async move {
                match changed {
                    Some(changed) => changed.await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                _ = waiter.woken() => break,
                () = facts_changed => continue,
                () = tokio::time::sleep_until(deadline) => break,
            }
        }
        // Woken, replaced, closing or timed out: one final answer with a fresh lease.
        let response = self.read(token, &request).await?.0;
        Ok(answered(&waiter, response))
    }

    /// Bytes of a rule-set artifact some netmap of the calling Node
    /// references while it can still be under a lease. Anything else, and an
    /// unknown digest, is `NotFound`.
    pub async fn artifact(&self, token: &str, digest: String) -> Result<Vec<u8>, Error> {
        let token = proof::token_hash(token)?;
        let digest: meshway_rules::Digest = digest.parse().map_err(|_| Error::NotFound)?;
        let (sealer, boot) = (self.sealer.clone(), self.boot);
        let validity = i64::try_from(self.timing.validity.as_millis())
            .map_err(|_| Error::Internal("netmap validity"))?;
        self.handle
            .read(move |tx| {
                let now = crate::store::execution::now()?;
                let principal =
                    crate::store::execution::authorize_node(tx, &sealer, boot, token, now)?;
                if !crate::store::resources::artifacts::referenced(
                    tx,
                    principal.network.as_uuid().as_bytes(),
                    principal.node.as_uuid().as_bytes(),
                    &digest,
                    now,
                    validity,
                )? {
                    return Err(Error::NotFound);
                }
                crate::store::resources::artifacts::load(tx, &digest)
            })
            .await
    }

    async fn read(
        &self,
        token: [u8; 32],
        request: &SyncRequest,
    ) -> Result<(SyncResponse, Option<PeerFactsChanged>), Error> {
        let (sealer, boot, facts, signer, validity, associations) = (
            self.sealer.clone(),
            self.boot,
            self.facts.clone(),
            self.signer.clone(),
            self.timing.validity,
            self.associations.clone(),
        );
        let request = request.clone();
        self.handle
            .read(move |tx| {
                sync::read(
                    tx,
                    &sealer,
                    boot,
                    token,
                    &request,
                    facts.as_ref(),
                    &sync::Issue {
                        signer: &signer,
                        validity,
                        associations: &associations,
                    },
                )
            })
            .await
    }
}
