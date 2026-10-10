//! Idempotent application of Engine's desired content. The Manager diffs its
//! actual resources and selected channels against the new plan in one pass:
//! unchanged ones stay, the rest retire, new ones are installed. Owners read
//! the current plan when they act; none waits for or adopts new content.
use super::*;
use meshway_model::state::ResourceRef;

impl Shared {
    pub(super) fn register_link_socket(&self, fact: &LinkFact) -> Result<(), Error> {
        let resource = if fact.carrier.listener == self.plan().node {
            ResourceRef::Listener {
                key: fact.carrier.listener_key.clone(),
            }
        } else {
            ResourceRef::Dial {
                key: format!("{}/{}", fact.carrier.listener, fact.carrier.listener_key),
            }
        };
        self.register_socket(fact.socket_id, resource, fact.carrier.protocol, fact.local)
    }
    pub(super) fn register_socket(
        &self,
        id: [u8; 32],
        resource: ResourceRef,
        protocol: TransportProtocol,
        local: SocketAddr,
    ) -> Result<(), Error> {
        let Some(reference) = traversal::socket_reference(self, id, resource)? else {
            // No live instance yet: registered again once one is bound.
            return Ok(());
        };
        self.sockets
            .lock()
            .map_err(|_| Error::Worker)?
            .entry(id)
            .or_insert(SocketFact {
                reference,
                network: if protocol == TransportProtocol::NativeUdp {
                    meshway_model::state::SocketNetwork::Udp
                } else {
                    meshway_model::state::SocketNetwork::Tcp
                },
                bound_address: local,
                endpoints: Vec::new(),
            });
        Ok(())
    }
    pub(super) fn remove_socket(&self, id: [u8; 32]) {
        if let Ok(mut sockets) = self.sockets.lock() {
            sockets.remove(&id);
        }
    }

    /// Whether a selected channel survives from `previous` to `next`: exact
    /// peer keys, unchanged physical resources, a still admitted carrier
    /// that still leads this Node's carrier order, and live health.
    fn keeps(&self, previous: &Plan, next: &Plan, link: &Link) -> bool {
        let fact = &link.fact;
        if !previous.peer_unchanged(next, fact.peer)
            || next.admit(fact.peer, &fact.carrier).is_err()
        {
            return false;
        }
        let resource = self
            .sockets
            .lock()
            .ok()
            .and_then(|sockets| {
                sockets
                    .get(&fact.socket_id)
                    .map(|socket| socket.reference.resource.clone())
            })
            .unwrap_or_else(|| {
                if fact.carrier.listener == previous.node {
                    ResourceRef::Listener {
                        key: fact.carrier.listener_key.clone(),
                    }
                } else {
                    ResourceRef::Dial {
                        key: format!("{}/{}", fact.peer, fact.carrier.listener_key),
                    }
                }
            });
        let dial = ResourceRef::Dial {
            key: format!("{}/{}", fact.peer, fact.carrier.listener_key),
        };
        // Exact resource equality includes TLS, proxy credentials and WS
        // settings; a changed physical declaration never keeps old I/O.
        let compatible = |resource: &ResourceRef| previous.netmap_compatible(next, resource);
        if !compatible(&resource)
            || (fact.carrier.dialer == previous.node && !compatible(&dial))
            || link.scope.check().is_err()
        {
            return false;
        }
        // A changed carrier order reselects unless this carrier still leads.
        fact.carrier.dialer != next.node
            || next
                .outgoing(fact.peer)
                .ok()
                .and_then(|candidates| {
                    candidates
                        .first()
                        .map(|(_, dial)| dial.listener_key.clone())
                })
                .is_some_and(|key| key == fact.carrier.listener_key)
    }

    /// One pass over the selected-channel table and socket references.
    pub(super) fn retain_table(&self, previous: &Plan, next: &Plan) -> Result<(), Error> {
        for socket in self.sockets.lock().map_err(|_| Error::Worker)?.values_mut() {
            socket.reference.netmap = next.reference;
        }
        let mut links = self.links.lock().map_err(|_| Error::Worker)?;
        let peers: Vec<_> = links.keys().copied().collect();
        for peer in peers {
            let keep = links
                .get(&peer)
                .is_some_and(|link| self.keeps(previous, next, link));
            if !keep && let Some(link) = links.remove(&peer) {
                link.scope.cancellation.cancel();
                diagnostic_link(
                    self,
                    &link.fact,
                    &link.scope,
                    "retire",
                    "content_incompatible",
                    None,
                );
            }
        }
        // install_ready rechecks the current content, so a departed peer's
        // late READY cannot observe this as a vacant slot.
        links.last.retain(|peer, _| next.has_peer(*peer));
        Ok(())
    }
}

impl Manager {
    /// Apply new desired content. Same content is a no-op. Unchanged own
    /// identity keeps every compatible resource and selected channel and
    /// replaces only what changed; a different own identity, address, key or
    /// pool restarts the Manager. Retired owners are joined before return.
    pub async fn apply(&mut self, desired: &Desired) -> Result<(), Error> {
        let previous = self.shared.plan();
        if self.shared.stop.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if previous.reference == desired.reference() {
            return Ok(());
        }
        let next = Plan::new(desired)?;
        validate_scale(&next, self.shared.limits)?;
        if !previous.same_self(&next) {
            return self.restart(desired).await;
        }
        let retired = self.retire_resources(&previous, &next)?;
        // Queued frames stay: receive keeps only frames of a still selected
        // channel, and SecureNode admits each on the current content.
        // Publishing first makes every later commit check the new content at
        // install_ready; the table pass below then sees the final channels.
        self.shared.plans.send_replace(next.clone());
        self.shared.retain_table(&previous, &next)?;
        self.shared.interests.allow(&next);
        self.join_retired(retired).await?;
        self.rebind_sockets()?;
        self.install_resources().await
    }

    /// Socket authorization of kept traversal descriptors names the new
    /// content's State reference; earlier offers and discovery end there.
    fn rebind_sockets(&self) -> Result<(), Error> {
        let endpoints: Vec<_> = self
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .iter()
            .map(|(id, endpoint)| (*id, endpoint.authorization.clone()))
            .collect();
        for (id, authorization) in endpoints {
            let reference = self
                .shared
                .sockets
                .lock()
                .map_err(|_| Error::Worker)?
                .get(&id)
                .map(|fact| fact.reference.clone());
            if let Some(reference) = reference {
                authorization.rebind(&reference)?;
            }
        }
        Ok(())
    }

    async fn restart(&mut self, desired: &Desired) -> Result<(), Error> {
        let traversal = self
            .traversal
            .as_ref()
            .map(|(gateway, generation)| {
                generation
                    .checked_next()
                    .map(|generation| (gateway.clone(), generation))
                    .map_err(|_| Error::Capacity)
            })
            .transpose()?;
        let carried = Carried {
            interests: self.shared.interests.clone(),
            facts: self.shared.facts.clone(),
            instance: self
                .shared
                .instance
                .lock()
                .map_err(|_| Error::Worker)?
                .clone(),
        };
        let keys = self.shared.keys.clone();
        self.shutdown().await?;
        *self = Self::start_inner(
            desired,
            keys,
            self.factory.clone(),
            self.shared.limits,
            traversal,
            Some(carried),
        )
        .await?;
        Ok(())
    }
}
