//! Explicit Control-approved handoff of physical resources. Each operation's
//! Scope stays immutable; only the owner takes a fresh scope for future work.
use super::*;
use meshway_authority::ExecutionSnapshot;
use tokio::sync::watch;

impl Shared {
    pub(super) fn register_link_socket(&self, fact: &LinkFact) -> Result<(), Error> {
        let cfg = self
            .identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        let resource = if fact.carrier.listener == cfg.node_id {
            meshway_model::state::ResourceRef::Listener {
                key: fact.carrier.listener_key.clone(),
            }
        } else {
            meshway_model::state::ResourceRef::Dial {
                key: format!("{}/{}", fact.carrier.listener, fact.carrier.listener_key),
            }
        };
        self.register_socket(fact.socket_id, resource, fact.carrier.protocol, fact.local)
    }
    pub(super) fn register_socket(
        &self,
        id: [u8; 32],
        resource: meshway_model::state::ResourceRef,
        protocol: TransportProtocol,
        local: SocketAddr,
    ) -> Result<(), Error> {
        let reference = traversal::socket_reference(&self.identity, id, self.generation, resource)?;
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
        if let Ok(mut epochs) = self.socket_epochs.lock() {
            epochs.remove(&id);
        }
    }
    pub(super) fn epoch_changed(&self) -> bool {
        let current = self.epochs.borrow();
        !same(current.snapshot(), self.identity.snapshot())
    }
    pub(super) async fn latest(
        &self,
        updates: &mut watch::Receiver<DataPlaneIdentity>,
    ) -> Result<Arc<Self>, Error> {
        loop {
            if self.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let next = updates.borrow_and_update().clone();
            if next.snapshot().ensure_current().is_ok() {
                return Ok(self.rebind(next));
            }
            let deadline = self
                .identity
                .snapshot()
                .retention_deadline()
                .map_err(|_| Error::Authority)?;
            tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::Cancelled),
                _ = tokio::time::sleep_until(deadline.into()) => return Err(Error::Authority),
                result = updates.changed() => { result.map_err(|_| Error::Cancelled)?; },
            }
        }
    }

    pub(super) fn listener_handoff(&self, id: [u8; 32]) -> Result<(), Error> {
        self.identity
            .snapshot()
            .ensure_current()
            .map_err(|_| Error::Authority)?;
        if let Some(fact) = self.sockets.lock().map_err(|_| Error::Worker)?.get_mut(&id) {
            fact.reference.configuration = self.identity.snapshot().reference();
        }
        self.socket_epochs
            .lock()
            .map_err(|_| Error::Worker)?
            .insert(id, self.identity.snapshot().clone());
        Ok(())
    }
    fn rebind(&self, identity: DataPlaneIdentity) -> Arc<Self> {
        self.rebind_scoped(identity, self.stop.clone())
    }
    pub(super) fn for_owner(&self) -> Arc<Self> {
        self.rebind_scoped(self.identity.clone(), self.stop.child_token())
    }
    fn rebind_scoped(&self, identity: DataPlaneIdentity, stop: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            identity,
            stop,
            links: self.links.clone(),
            events: self.events.clone(),
            control_events: self.control_events.clone(),
            capacity: self.capacity.clone(),
            limits: self.limits,
            control_budget: self.control_budget.clone(),
            sockets: self.sockets.clone(),
            epochs: self.epochs.clone(),
            gates: self.gates.clone(),
            socket_epochs: self.socket_epochs.clone(),
            generation: self.generation,
            traversal_endpoints: self.traversal_endpoints.clone(),
            resource_changes: self.resource_changes.clone(),
        })
    }

    pub(super) fn physical(&self) -> Result<Arc<scope::Physical>, Error> {
        let value = Arc::new(scope::Physical::new(
            self.epochs.subscribe(),
            self.stop.clone(),
        ));
        let mut gates = self.gates.lock().map_err(|_| Error::Worker)?;
        gates.retain(|gate| gate.strong_count() != 0);
        if gates.len()
            >= self.limits.connections * 4 + self.limits.listeners * 2 + self.limits.candidates * 2
        {
            return Err(Error::Capacity);
        }
        gates.push(Arc::downgrade(&value));
        Ok(value)
    }

    pub(super) async fn successor(
        &self,
        updates: &mut watch::Receiver<DataPlaneIdentity>,
    ) -> Result<Arc<Self>, Error> {
        loop {
            if self.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let next = updates.borrow_and_update().clone();
            if !next.snapshot().same_execution(self.identity.snapshot())
                || next.snapshot().expires_at() != self.identity.snapshot().expires_at()
            {
                next.snapshot()
                    .continuation_from(self.identity.snapshot())
                    .map_err(|_| Error::Authority)?;
                return Ok(self.rebind(next));
            }
            let deadline = self
                .identity
                .snapshot()
                .retention_deadline()
                .map_err(|_| Error::Authority)?;
            tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::Cancelled),
                _ = tokio::time::sleep_until(deadline.into()) => return Err(Error::Authority),
                result = updates.changed() => { result.map_err(|_| Error::Cancelled)?; },
            }
        }
    }

    pub(super) fn renew_link(&self, fact: &LinkFact) -> Result<Option<Scope>, Error> {
        let mut links = self.links.lock().map_err(|_| Error::Worker)?;
        let Some(link) = links
            .get_mut(&fact.peer)
            .filter(|link| link.fact.id == fact.id)
        else {
            return Ok(None);
        };
        let Ok(previous) = self
            .identity
            .snapshot()
            .continuation_from(&link.scope.snapshot)
        else {
            return Ok(None);
        };
        let next = self
            .identity
            .snapshot()
            .configuration()
            .map_err(|_| Error::Authority)?;
        let resource = self
            .sockets
            .lock()
            .map_err(|_| Error::Worker)?
            .get(&fact.socket_id)
            .map(|socket| socket.reference.resource.clone())
            .unwrap_or_else(|| {
                if fact.carrier.listener == previous.node_id {
                    meshway_model::state::ResourceRef::Listener {
                        key: fact.carrier.listener_key.clone(),
                    }
                } else {
                    meshway_model::state::ResourceRef::Dial {
                        key: format!("{}/{}", fact.peer, fact.carrier.listener_key),
                    }
                }
            });
        if previous.peers.get(&fact.peer) != next.peers.get(&fact.peer)
            || !next.peers.contains_key(&fact.peer)
            // Exact resource equality includes TLS, proxy credentials and WS
            // settings. A provider's buffered writes retain Session's frozen
            // output scope; changing the physical epoch never renews old I/O.
            || !meshway_underlay::resource_compatible(previous, next, &resource)
            || (fact.carrier.dialer == previous.node_id
                && !meshway_underlay::resource_compatible(
                    previous,
                    next,
                    &meshway_model::state::ResourceRef::Dial {
                        key: format!("{}/{}", fact.peer, fact.carrier.listener_key),
                    },
                ))
            || link.scope.health.as_ref().is_some_and(|health| {
                health
                    .lock()
                    .map_or(true, |deadline| tokio::time::Instant::now() >= *deadline)
            })
        {
            return Ok(None);
        }
        let mut scope = self.scope(Some((fact.peer, fact.carrier.clone())), false);
        scope.health = link.scope.health.clone();
        if scope.check().is_err() {
            return Ok(None);
        }
        link.scope.cancellation.cancel();
        link.scope = scope.clone();
        link.batch_scope = meshway_transport::ChannelScope::new();
        if let Some(socket) = self
            .sockets
            .lock()
            .map_err(|_| Error::Worker)?
            .get_mut(&fact.socket_id)
        {
            socket.reference.configuration = self.identity.snapshot().reference();
        }
        Ok(Some(scope))
    }
}

impl Manager {
    /// Whether Control authorizes a differential physical handoff. Individual
    /// resources and channels are retained only after their exact proof checks.
    pub fn can_reconcile(&self, identity: &DataPlaneIdentity) -> bool {
        if self.shared.stop.is_cancelled() {
            return false;
        }
        let next = identity.snapshot();
        let Ok(old) = next.continuation_from(self.shared.identity.snapshot()) else {
            return false;
        };
        let Ok(cfg) = next.configuration() else {
            return false;
        };
        if validate_scale(cfg, self.shared.limits).is_err()
            || old.mesh_address_pools != cfg.mesh_address_pools
        {
            return false;
        }
        true
    }

    pub(super) async fn retain(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        if !self.can_reconcile(&identity) {
            return Err(Error::Authority);
        }
        let previous = self.shared.identity.snapshot().clone();
        let next = identity.snapshot().clone();
        // Do not manufacture an epoch change when the offered authority is
        // literally unchanged (otherwise waiting owners would await forever).
        if next.same_execution(&previous) && next.expires_at() == previous.expires_at() {
            return Ok(());
        }
        let retired = self.retire_resources(
            next.continuation_from(&previous)
                .map_err(|_| Error::Authority)?,
            next.configuration().map_err(|_| Error::Authority)?,
        );
        self.shared = self.shared.rebind(identity.clone());
        let generation = self
            .shared
            .generation
            .checked_next()
            .map_err(|_| Error::Capacity)?;
        Arc::get_mut(&mut self.shared)
            .ok_or(Error::Worker)?
            .generation = generation;
        if let Some((_, current)) = &mut self.traversal {
            *current = generation;
        }
        while self.events.try_recv().is_ok() {}
        while self.control_events.try_recv().is_ok() {}
        self.shared.epochs.send_replace(identity);
        for gate in self
            .shared
            .gates
            .lock()
            .map_err(|_| Error::Worker)?
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            gate.wake();
        }
        self.join_retired(retired).await?;
        self.install_resources().await?;
        let deadline = previous
            .retention_deadline()
            .map_err(|_| Error::Authority)?;
        loop {
            next.ensure_current().map_err(|_| Error::Authority)?;
            let links_ready = self
                .shared
                .links
                .lock()
                .map_err(|_| Error::Worker)?
                .values()
                .all(|link| same(&link.scope.snapshot, &next));
            let sockets_ready = self
                .shared
                .sockets
                .lock()
                .map_err(|_| Error::Worker)?
                .values()
                .all(|socket| socket.reference.configuration == next.reference());
            let owners_ready = self
                .shared
                .socket_epochs
                .lock()
                .map_err(|_| Error::Worker)?
                .values()
                .all(|snapshot| same(snapshot, &next));
            if links_ready && sockets_ready && owners_ready {
                // install_ready rejects every previous acquisition epoch, even
                // still-live same-target snapshots. No departed peer's late
                // READY can observe this collection as a new vacant slot.
                let cfg = next.configuration().map_err(|_| Error::Authority)?;
                self.shared
                    .links
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .last
                    .retain(|peer, _| cfg.peers.contains_key(peer));
                return Ok(());
            }
            tokio::select! { biased;
                _ = self.shared.stop.cancelled() => return Err(Error::Cancelled),
                _ = next.invalidated() => return Err(Error::Authority),
                _ = tokio::time::sleep_until(deadline.into()) => return Err(Error::Authority),
                _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => {},
            }
        }
    }
}

pub(super) fn same(a: &ExecutionSnapshot, b: &ExecutionSnapshot) -> bool {
    a.same_execution(b) && a.expires_at() == b.expires_at()
}
