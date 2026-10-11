//! Netmap distribution: the long-poll `POST /agent-api/v1/sync`.
//!
//! A sync request names the netmap and peer-fact cursor the Node holds. The
//! Server answers at once when either is behind; otherwise it holds the
//! request until a committed change wakes it, the peer-fact source reports a
//! change or a traversal attempt the Node does not hold yet, a newer request
//! of the same Node replaces it, or the hold times out. Every answer carries a fresh lease for the current netmap. Content is
//! stored by `store::netmap` and only changes in stable-input transactions.
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
}

impl Netmaps {
    pub(crate) fn new(
        handle: Handle,
        sealer: Arc<Sealer>,
        boot: InstanceId,
        hub: Arc<Hub>,
        facts: Arc<dyn PeerFactsSource>,
        signer: Arc<signing::NetmapSigningKeys>,
        timing: SyncTiming,
    ) -> Self {
        Self {
            handle,
            sealer,
            boot,
            hub,
            facts,
            signer,
            timing,
        }
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
        let answered = |response: SyncResponse| {
            records::Records::shared().issued(principal.network, principal.node, response.lease);
            response
        };
        // Register before the first read: a change committed after this point
        // wakes the waiter, one committed before it is visible to the read.
        let mut waiter = self.hub.register(principal.network, principal.node)?;
        let deadline = tokio::time::Instant::now() + self.timing.jittered_hold();
        loop {
            let (response, changed) = self.read(token, &request).await?;
            if !matches!(
                response.netmap,
                meshway_protocol::netmap::SyncNetmap::Unchanged
            ) || response
                .peer_facts
                .as_ref()
                .is_some_and(|facts| facts.base.is_none() || Some(facts.cursor) != request.state)
                || response.traversal_requests.as_slice().iter().any(|asked| {
                    !request
                        .traversal_requests
                        .as_slice()
                        .contains(&asked.attempt_id)
                })
            {
                return Ok(answered(response));
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
        Ok(answered(self.read(token, &request).await?.0))
    }

    async fn read(
        &self,
        token: [u8; 32],
        request: &SyncRequest,
    ) -> Result<(SyncResponse, Option<PeerFactsChanged>), Error> {
        let (sealer, boot, facts, signer, validity) = (
            self.sealer.clone(),
            self.boot,
            self.facts.clone(),
            self.signer.clone(),
            self.timing.validity,
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
                    },
                )
            })
            .await
    }
}
