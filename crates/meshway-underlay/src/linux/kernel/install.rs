//! Installing desired content (TUN, capture, negative boundary), keeping
//! what new content for the same identity leaves unchanged, and suspending.
use super::*;

impl Kernel {
    pub async fn install(&mut self, desired: Desired) -> Result<()> {
        self.ready().await?;
        // The same content is already installed: nothing changes.
        if self.tun.is_some()
            && self
                .shared
                .installed()?
                .is_some_and(|installed| installed.same(&desired))
        {
            return Ok(());
        }
        let view = View::of(&desired);
        if self.exit.as_ref().is_some_and(|owner| {
            view.exit_role().as_ref() != Some(&owner.role.plan) || !view.exit_enabled()
        }) {
            self.release_exit().await?;
        }
        let binding = (view.network_id(), view.node_id());
        if self
            .journal
            .binding
            .is_some_and(|previous| previous != binding)
        {
            return Err(Error::Target);
        }
        match view.egress_capture() {
            Some(plan)
                if plan.default_capture
                    || !plan.managed_ipv4.is_empty()
                    || !plan.sink_ipv6.is_empty()
                    || plan.unavailable.is_some()
                    || self.egress.is_some() =>
            {
                self.prepare_egress(plan.clone()).await?;
            }
            // A disabled netmap Node owns no capture; keep it negative.
            None => {
                if let Some(owner) = &self.egress {
                    owner.capture.pause().await?;
                }
            }
            Some(_) => {}
        }
        if view
            .egress_capture()
            .is_some_and(|plan| plan.unavailable.is_some())
        {
            return Err(Error::Target);
        }
        for candidate in view.dialers().values() {
            validate_dial(&candidate.parameters)?;
        }
        for listener in view.listeners().values() {
            if listener_enabled(listener.declaration()) {
                validate_listener(listener.declaration())?;
            }
        }
        let pools = plan::pools(
            view.mesh_address_pools(),
            view.addresses(),
            self.options.max_pools,
        )?;
        if self.journal.anchor.as_ref().is_some_and(|anchor| {
            pools
                .iter()
                .any(|pool| pool.contains(&IpAddr::V4(anchor.identity.address)))
        }) {
            return Err(Error::Conflict);
        }
        let addresses = view.addresses().clone();
        self.verify_capture(&pools).await?;
        if self.retain_tun(&desired, &pools, &addresses).await? {
            self.resume_egress().await?;
            return Ok(());
        }
        self.verify().await?;
        self.journal.binding = Some(binding);
        self.store.persist(&self.journal).await?;
        // Fence the old packet/carrier scope, then protect the complete new
        // pool set before removing any old positive routes or the owned TUN.
        // Retired pools keep their negative rules until explicit down.
        self.shared.suspend()?;
        self.release_exit().await?;
        self.retain_negative(&pools).await?;
        self.suspend().await?;
        let outcome = self.activate(&desired, &pools, &addresses).await;
        if outcome.is_err() {
            if self.journal.pending.is_none() {
                // Known creations can be rolled back without turning a
                // cancelled installation into a protection conflict.
                if let Err(error) = self.suspend().await {
                    self.end_after("install", &error);
                    return Err(error);
                }
            } else {
                self.shared.suspend()?;
                if let Err(error) = &outcome {
                    self.end_after("install", error);
                }
            }
            // Descriptor ownership makes dropping this new device safe even if
            // an interrupted netlink operation has an uncertain outcome.
            self.drop_owned_tun();
        }
        outcome?;
        self.resume_egress().await
    }

    /// New content for the same own identity and pools keeps the host IP
    /// interface and every descriptor whose exact purpose is unchanged.
    async fn retain_tun(
        &self,
        desired: &Desired,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<bool> {
        let Some(tun) = &self.tun else {
            return Ok(false);
        };
        let Some(previous) = self.shared.installed()? else {
            return Ok(false);
        };
        let Some(old) = continuation(desired, &previous) else {
            return Ok(false);
        };
        let old_pools = plan::pools(
            old.mesh_address_pools(),
            old.addresses(),
            self.options.max_pools,
        )?;
        if !same_pools(&old_pools, pools) {
            return Ok(false);
        }

        self.verify().await?;
        self.verify_tun(tun, pools, addresses).await?;
        self.quiesce().await?;
        self.shared.check()?;
        let mut active = self.shared.execution.write().map_err(|_| Error::Worker)?;
        let Some(installed) = active.as_mut() else {
            return Err(Error::Expired);
        };
        if !installed.desired.same(&previous) {
            return Err(Error::Expired);
        }
        self.shared.resources.apply(&previous, desired)?;
        installed.desired = desired.clone();
        Ok(true)
    }

    /// Only a negative boundary. This creates no TUN, socket permit, or grant.
    pub(super) async fn retain_negative(&mut self, pools: &[ipnet::IpNet]) -> Result<()> {
        self.ready().await?;
        let mut union = self.journal.pools.clone();
        for pool in pools {
            if !union.contains(pool) {
                union.push(*pool);
            }
        }
        if union.len() > self.journal.pool_limit {
            return Err(Error::Capacity);
        }
        // Arm before the dump and before any new rule: a concurrent addition
        // cannot hide between preflight and publication. Retired pools stay armed.
        self.quiesce().await?;
        self.expected.lock().map_err(|_| Error::Worker)?.pools = union.clone();
        self.verify_capture(&union).await?;
        self.quiesce().await?;
        self.journal.pools = union;
        self.store.persist(&self.journal).await?;
        if self.journal.pools.is_empty() {
            // Explicitly empty pools are valid configuration. There is no
            // capture block to fabricate, nor any retired boundary to preserve.
            return self.verify().await;
        }
        for ipv6 in [false, true] {
            // End must exist before installing a goto that targets it.
            self.add_rule(RuleSpec {
                ipv6,
                priority: self.journal.end(),
                table: self.journal.table,
                kind: RuleKind::End,
            })
            .await?;
            self.add_rule(RuleSpec {
                ipv6,
                priority: self.journal.start,
                table: self.journal.table,
                kind: RuleKind::Bypass {
                    mark: self.journal.mark,
                    end: self.journal.end(),
                },
            })
            .await?;
        }
        let union = self.journal.pools.clone();
        for (index, pool) in union.iter().enumerate() {
            let preference = self.journal.start + 1 + (index as u32) * 2;
            // Negative before positive lookup, including during partial install.
            self.add_rule(RuleSpec {
                ipv6: pool.addr().is_ipv6(),
                priority: preference + 1,
                table: self.journal.table,
                kind: RuleKind::Sink { pool: *pool },
            })
            .await?;
            self.add_rule(RuleSpec {
                ipv6: pool.addr().is_ipv6(),
                priority: preference,
                table: self.journal.table,
                kind: RuleKind::Lookup { pool: *pool },
            })
            .await?;
        }
        self.quiesce().await?;
        self.verify().await
    }

    async fn activate(
        &mut self,
        desired: &Desired,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<()> {
        self.journal.pending = Some(Pending::CreateTun);
        self.store.persist(&self.journal).await?;
        let mut options = meshway_tun::Options::new(self.options.mtu)?.with_batch_io(true);
        if let Some(ip) = addresses.ipv4 {
            options = options.with_ipv4(ip, 32)?;
        }
        if let Some(ip) = addresses.ipv6 {
            options = options.with_ipv6(ip, 128)?;
        }
        let created = {
            // Creation is synchronous. Hold the monitor projection lock until
            // the live descriptor's exact address exception is installed; even
            // another Tokio worker cannot classify our creation as foreign.
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.changing = true;
            let created = Tun::create(options).and_then(|tun| {
                let index = tun.if_index()?;
                Ok((tun, index))
            });
            if let Ok((tun, index)) = &created {
                expected.tun = Some((
                    *index,
                    tun.name().to_owned(),
                    tun.mtu(),
                    [
                        addresses.ipv4.map(IpAddr::V4),
                        addresses.ipv6.map(IpAddr::V6),
                    ]
                    .into_iter()
                    .flatten()
                    .collect(),
                ));
            }
            created
        };
        let (tun, index) = match created {
            Ok(created) => created,
            Err(error) => {
                // meshway-tun drops every failed creation's owned descriptor.
                // This known failure needs no guessed kernel cleanup. Independently
                // observed capture conflicts still fence the socket owner.
                self.journal.pending = None;
                self.expected.lock().map_err(|_| Error::Worker)?.changing = false;
                self.store.persist(&self.journal).await?;
                return Err(error.into());
            }
        };
        self.tun = Some(tun);
        let tun = self.tun.as_ref().ok_or(Error::Worker)?;
        self.journal.tun = Some(TunRecord {
            name: tun.name().to_owned(),
            ifindex: index,
            mtu: tun.mtu(),
            addresses: addresses.clone(),
        });
        self.journal.pending = None;
        self.store.persist(&self.journal).await?;
        self.address_ready(index, addresses).await?;
        for route in plan::positive_routes(pools, addresses, self.journal.table, index) {
            self.add_route(route).await?;
        }
        self.quiesce().await?;
        self.expected.lock().map_err(|_| Error::Worker)?.changing = false;
        self.verify().await?;
        self.quiesce().await?;
        self.shared.check()?;
        *self.shared.execution.write().map_err(|_| Error::Worker)? = Some(
            super::super::protection::Installed::new(desired.clone(), CancellationToken::new()),
        );
        Ok(())
    }

    pub(super) async fn address_ready(
        &self,
        index: u32,
        wanted: &meshway_configuration::NodeAddresses,
    ) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let addresses = get_addresses(&self.handle, self.options.max_kernel_objects).await?;
                let ready = [wanted.ipv4.map(IpAddr::V4), wanted.ipv6.map(IpAddr::V6)].into_iter().flatten().all(|wanted| addresses.iter().any(|address| {
                    // IFA_F_TENTATIVE=0x40, IFA_F_DADFAILED=0x08. Check the
                    // extended flags as well as the legacy header byte.
                    let flags = network::address_flags(address);
                    address.header.index == index && flags & 0x48 == 0 && address.header.prefix_len == if wanted.is_ipv4() { 32 } else { 128 }
                        && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(ip) | AddressAttribute::Local(ip) if *ip == wanted))
                }));
                if ready { return Ok(()); }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.map_err(|_| Error::Target)?
    }

    pub async fn suspend(&mut self) -> Result<()> {
        self.shared.suspend()?;
        if let Some(owner) = &self.egress {
            owner.capture.pause().await?;
        }
        self.release_exit().await?;
        let result = self.remove_live().await;
        self.drop_owned_tun();
        // Closing the owned descriptor emits address/link notifications after
        // its positive capture exception has been removed. Drain with the
        // observation tombstone still present, including cancellation retries.
        let drained = self.quiesce().await;
        if drained.is_ok() {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .retiring_tun = None;
        }
        result?;
        drained?;
        self.journal.tun = None;
        self.store.persist(&self.journal).await
    }

    pub(super) fn drop_owned_tun(&mut self) {
        if let Ok(mut expected) = self.expected.lock() {
            if let Some((index, _, _, _)) = expected.tun.take() {
                expected.retiring_tun = Some(index);
            }
            expected.changing = false;
        }
        self.tun.take();
    }

    async fn remove_live(&mut self) -> Result<()> {
        self.ready().await?;
        self.verify().await?;
        while let Some(record) = self.journal.routes.last().cloned() {
            self.verify().await?;
            self.journal.pending = Some(Pending::DeleteRoute {
                record: record.clone(),
            });
            self.store.persist(&self.journal).await?;
            let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
            self.objects()
                .remove_route(&record.observed, &routes)
                .await?;
            self.journal.routes.pop();
            self.journal.pending = None;
            self.store.persist(&self.journal).await?;
        }
        self.quiesce().await
    }
}

/// The installed `previous` content as a view, when `next` is new content
/// for the same Node of the same network. Comparison data only.
fn continuation<'a>(next: &Desired, previous: &'a Desired) -> Option<View<'a>> {
    (!next.same(previous)
        && next.netmap().network_id == previous.netmap().network_id
        && next.netmap().node_id == previous.netmap().node_id
        && next.local().self_keys == previous.local().self_keys)
        .then(|| View::of(previous))
}

pub(super) fn same_pools(left: &[ipnet::IpNet], right: &[ipnet::IpNet]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}
