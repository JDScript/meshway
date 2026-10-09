//! Explicit Control-approved handoff of physical resources. Each operation's
//! Scope stays immutable. When a continuing epoch is installed, the Manager
//! itself renews or retires every selected channel and socket reference in
//! one pass; owners only adopt that result. Only owners holding external
//! socket authorization report a completion event the Manager waits for.
use super::*;
use meshway_authority::ExecutionSnapshot;
use meshway_model::{Sequence, state::ResourceRef};
use tokio::sync::watch;

impl EpochContext {
    pub(super) fn register_link_socket(&self, fact: &LinkFact) -> Result<(), Error> {
        let resource = if fact.carrier.listener == self.plan.node {
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
        if self.plan.on_demand() {
            // See Manager::register_listener: no netmap State socket reference.
            return Ok(());
        }
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
        self.handoffs
            .send_if_modified(|done| done.remove(&id).is_some());
    }
    pub(super) fn epoch_changed(&self) -> bool {
        !same(
            self.epochs.borrow().identity.snapshot(),
            self.identity.snapshot(),
        )
    }
    pub(super) async fn latest(
        &self,
        updates: &mut watch::Receiver<Epoch>,
    ) -> Result<Arc<Self>, Error> {
        loop {
            if self.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let next = updates.borrow_and_update().clone();
            if next.identity.snapshot().ensure_current().is_ok() {
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

    fn rebind(&self, epoch: Epoch) -> Arc<Self> {
        self.rebind_scoped(epoch, self.stop.clone())
    }
    pub(super) fn for_owner(&self) -> Arc<Self> {
        self.rebind_scoped(
            Epoch {
                identity: self.identity.clone(),
                plan: self.plan.clone(),
                generation: self.generation,
            },
            self.stop.child_token(),
        )
    }
    fn rebind_scoped(&self, epoch: Epoch, stop: CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            identity: epoch.identity,
            plan: epoch.plan,
            stop,
            links: self.links.clone(),
            events: self.events.clone(),
            control_events: self.control_events.clone(),
            capacity: self.capacity.clone(),
            limits: self.limits,
            control_budget: self.control_budget.clone(),
            sockets: self.sockets.clone(),
            epochs: self.epochs.clone(),
            renewed: self.renewed.clone(),
            handoffs: self.handoffs.clone(),
            gates: self.gates.clone(),
            generation: epoch.generation,
            traversal_endpoints: self.traversal_endpoints.clone(),
            resource_changes: self.resource_changes.clone(),
            interests: self.interests.clone(),
            facts: self.facts.clone(),
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

    /// The next epoch directly continuing this one, once the Manager has
    /// renewed its selected-channel table. Same-content renewal never ends here.
    pub(super) async fn successor(
        &self,
        updates: &mut watch::Receiver<Epoch>,
    ) -> Result<Arc<Self>, Error> {
        loop {
            if self.stop.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let next = updates.borrow_and_update().clone();
            if !same(next.identity.snapshot(), self.identity.snapshot()) {
                if !continues(next.identity.snapshot(), self.identity.snapshot()) {
                    return Err(Error::Authority);
                }
                let mut renewed = self.renewed.subscribe();
                tokio::select! { biased;
                    _ = self.stop.cancelled() => return Err(Error::Cancelled),
                    result = renewed.wait_for(|done| *done >= next.generation) => {
                        result.map_err(|_| Error::Cancelled)?;
                    },
                }
                if !same(
                    self.epochs.borrow().identity.snapshot(),
                    next.identity.snapshot(),
                ) {
                    // A later epoch superseded the one we waited for.
                    return Err(Error::Authority);
                }
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

    /// Adopt the Manager's renewal of this owner's selected channel.
    pub(super) async fn adopt(
        &self,
        updates: &mut watch::Receiver<Epoch>,
        fact: &LinkFact,
    ) -> Result<(Arc<Self>, Scope), Error> {
        let next = self.successor(updates).await?;
        let scope = next.selected_scope(fact).ok_or(Error::Unavailable)?;
        Ok((next, scope))
    }

    /// The current scope of a still-selected channel installed in this epoch.
    pub(super) fn selected_scope(&self, fact: &LinkFact) -> Option<Scope> {
        let links = self.links.lock().ok()?;
        let link = links
            .get(&fact.peer)
            .filter(|link| link.fact.id == fact.id)?;
        (same(&link.scope.snapshot, self.identity.snapshot()) && link.scope.check().is_ok())
            .then(|| link.scope.clone())
    }

    /// Renew a selected channel for this (newly installed) epoch, or report
    /// that it must be retired. Exact peer keys, retained resources, a still
    /// admitted carrier, carrier preference and live health are required.
    fn renewal(&self, previous: &Plan, link: &Link) -> Option<Scope> {
        let fact = &link.fact;
        if !continues(self.identity.snapshot(), &link.scope.snapshot)
            || !previous.peer_unchanged(&self.plan, fact.peer)
        {
            return None;
        }
        let resource = self
            .sockets
            .lock()
            .ok()?
            .get(&fact.socket_id)
            .map(|socket| socket.reference.resource.clone())
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
        // settings. A provider's buffered writes retain Session's frozen
        // output scope; changing the physical epoch never renews old I/O.
        let compatible = |resource: &ResourceRef| {
            if self.plan.on_demand() {
                previous.netmap_compatible(&self.plan, resource)
            } else {
                let (Ok(old), Ok(new)) = (
                    self.identity
                        .snapshot()
                        .continuation_from(&link.scope.snapshot),
                    self.identity.snapshot().configuration(),
                ) else {
                    return false;
                };
                meshway_underlay::resource_compatible(old, new, resource)
            }
        };
        if !compatible(&resource)
            || (fact.carrier.dialer == previous.node && !compatible(&dial))
            || link.scope.health.as_ref().is_some_and(|health| {
                health
                    .lock()
                    .map_or(true, |deadline| tokio::time::Instant::now() >= *deadline)
            })
        {
            return None;
        }
        // A changed carrier order reselects unless this carrier still leads.
        if self.plan.on_demand()
            && fact.carrier.dialer == self.plan.node
            && self
                .plan
                .outgoing(fact.peer)
                .ok()?
                .first()
                .is_none_or(|(_, dial)| dial.listener_key != fact.carrier.listener_key)
        {
            return None;
        }
        let mut scope = self.scope(Some((fact.peer, fact.carrier.clone())), false);
        scope.health = link.scope.health.clone();
        scope.check().ok()?;
        Some(scope)
    }

    /// One pass over the selected-channel table and socket references.
    pub(super) fn renew_table(&self, previous: &Plan) -> Result<(), Error> {
        let next = self.identity.snapshot();
        if !self.plan.on_demand() {
            let reference = next.reference();
            for socket in self.sockets.lock().map_err(|_| Error::Worker)?.values_mut() {
                socket.reference.configuration = reference.clone();
            }
        }
        let mut links = self.links.lock().map_err(|_| Error::Worker)?;
        let peers: Vec<_> = links.keys().copied().collect();
        for peer in peers {
            let renewal = links
                .get(&peer)
                .and_then(|link| self.renewal(previous, link));
            match renewal {
                Some(scope) => {
                    let link = links.get_mut(&peer).ok_or(Error::Worker)?;
                    link.scope.cancellation.cancel();
                    link.scope = scope;
                    link.batch_scope = meshway_transport::ChannelScope::new();
                }
                None => {
                    if let Some(link) = links.remove(&peer) {
                        link.scope.cancellation.cancel();
                        diagnostic_link(
                            self,
                            &link.fact,
                            &link.scope,
                            "retire",
                            "handoff_incompatible",
                            None,
                        );
                    }
                }
            }
        }
        // install_ready already rejects every older acquisition epoch, so a
        // departed peer's late READY cannot observe this as a vacant slot.
        let plan = &self.plan;
        links.last.retain(|peer, _| plan.has_peer(*peer));
        Ok(())
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
        let current = self.shared.identity.snapshot();
        if next.is_netmap() && next.same_netmap(current) {
            return true;
        }
        if !continues(next, current) {
            return false;
        }
        let Ok(plan) = Plan::new(next) else {
            return false;
        };
        validate_scale(&plan, self.shared.limits).is_ok()
            && plan.mesh_address_pools == self.shared.plan.mesh_address_pools
    }

    pub(super) async fn retain(&mut self, identity: DataPlaneIdentity) -> Result<(), Error> {
        if !self.can_reconcile(&identity) {
            return Err(Error::Authority);
        }
        let previous = self.shared.identity.snapshot().clone();
        let next = identity.snapshot().clone();
        // Do not manufacture an epoch change when the offered authority is
        // literally unchanged (otherwise waiting owners would await forever).
        if same(&next, &previous) {
            return Ok(());
        }
        let plan = Plan::new(&next)?;
        let old_plan = self.shared.plan.clone();
        let retired = self.retire_resources(&old_plan, &plan, &previous, &next)?;
        let generation = self
            .shared
            .generation
            .checked_next()
            .map_err(|_| Error::Capacity)?;
        let epoch = Epoch {
            identity,
            plan: plan.clone(),
            generation,
        };
        self.shared = self.shared.rebind(epoch.clone());
        if let Some((_, current)) = &mut self.traversal {
            *current = generation;
        }
        while self.events.try_recv().is_ok() {}
        while self.control_events.try_recv().is_ok() {}
        // Publishing first fences every acquisition of an older epoch at
        // install_ready; the renewal below then sees the final table.
        self.shared.epochs.send_replace(epoch);
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
        self.shared.renew_table(&old_plan)?;
        self.shared.interests.allow(&plan);
        self.shared.renewed.send_replace(generation);
        self.join_retired(retired).await?;
        self.install_resources().await?;
        self.await_handoffs(&previous, generation).await
    }

    /// Owners that hold external socket authorization hand it to the new
    /// epoch themselves; wait for each one's completion event.
    async fn await_handoffs(
        &self,
        previous: &ExecutionSnapshot,
        generation: Sequence,
    ) -> Result<(), Error> {
        let expected: Vec<[u8; 32]> = self
            .shared
            .traversal_endpoints
            .lock()
            .map_err(|_| Error::Worker)?
            .keys()
            .copied()
            .collect();
        if expected.is_empty() {
            return Ok(());
        }
        let deadline = previous
            .retention_deadline()
            .map_err(|_| Error::Authority)?;
        let mut done = self.shared.handoffs.subscribe();
        let sockets = self.shared.sockets.clone();
        tokio::select! { biased;
            _ = self.shared.stop.cancelled() => Err(Error::Cancelled),
            _ = self.shared.identity.snapshot().invalidated() => Err(Error::Authority),
            _ = tokio::time::sleep_until(deadline.into()) => Err(Error::Authority),
            result = done.wait_for(|done| expected.iter().all(|id| {
                done.get(id).is_some_and(|at| *at >= generation)
                    // An owner that already exited released its socket.
                    || sockets.lock().is_ok_and(|sockets| !sockets.contains_key(id))
            })) => result.map(|_| ()).map_err(|_| Error::Cancelled),
        }
    }
}

/// Interchangeable authority: same netmap content, or the same publication
/// target and deadline.
pub(super) fn same(a: &ExecutionSnapshot, b: &ExecutionSnapshot) -> bool {
    if a.is_netmap() || b.is_netmap() {
        return a.same_netmap(b);
    }
    a.same_execution(b) && a.expires_at() == b.expires_at()
}

/// `next` directly continues `old` for the same identity and resources.
pub(super) fn continues(next: &ExecutionSnapshot, old: &ExecutionSnapshot) -> bool {
    match (next.is_netmap(), old.is_netmap()) {
        (true, true) => next.netmap_continuation_from(old).is_ok(),
        (false, false) => next.continuation_from(old).is_ok(),
        _ => false,
    }
}

/// Diagnostic authority label: publication, or netmap serial.
pub(super) fn label(snapshot: &ExecutionSnapshot) -> String {
    match snapshot.netmap_ref() {
        Some(netmap) => format!("netmap:{}", netmap.serial.get()),
        None => snapshot.reference().execution_publication.to_string(),
    }
}
