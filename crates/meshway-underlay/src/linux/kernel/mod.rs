mod anchor;
mod capture;
pub(crate) mod exit;
mod journal;
mod monitor;
mod plan;

use futures_util::{TryStream, TryStreamExt};
use meshway_authority::ExecutionSnapshot;
use meshway_private_storage::Directory;
use meshway_tun::Tun;
use rtnetlink::{
    Handle, IpVersion,
    packet_route::{
        AddressFamily,
        address::{AddressAttribute, AddressMessage},
        route::RouteMessage,
        rule::{RuleAttribute, RuleMessage},
    },
};
use std::{
    net::IpAddr,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::{
    protection::{Mark, Shared},
    sockets::{listener_enabled, validate_dial, validate_listener},
    tasks::Tasks,
};
use crate::{Error, Options, Result};
use journal::{Journal, Pending, RouteRecord, RuleRecord, Store, TunRecord};
use monitor::{Event, Expected};
use plan::{
    RouteSpec, RuleKind, RuleSpec, decode_route, decode_rule, encode_route, encode_rule, priority,
    route_table, rule_table,
};

pub(super) struct Kernel {
    handle: Handle,
    store: Store,
    journal: Journal,
    pub shared: Arc<Shared>,
    expected: Arc<Mutex<Expected>>,
    pub tun: Option<Tun>,
    options: Options,
    baseline: Vec<Vec<u8>>,
    dns_cancel: CancellationToken,
}

impl Kernel {
    pub fn owned_interfaces(&self) -> Vec<String> {
        self.journal
            .tun
            .iter()
            .map(|tun| tun.name.clone())
            .chain(
                self.journal
                    .anchor
                    .iter()
                    .map(|anchor| anchor.identity.name.clone()),
            )
            .collect()
    }

    pub async fn open(directory: Arc<Directory>, options: Options, tasks: Tasks) -> Result<Self> {
        // Subscribe before the baseline dump so rule changes cannot disappear
        // between discovery and the monitor's start.
        let monitor_socket = monitor::open()?;
        let (connection, handle, unsolicited) = rtnetlink::new_connection()?;
        drop(unsolicited); // request socket has no multicast subscriptions
        let conflict = CancellationToken::new();
        let failed = conflict.clone();
        tasks.spawn(async move {
            connection.await;
            failed.cancel();
        })?;
        let (mut store, previous) = Store::open(directory).await?;
        let rules = get_rules(&handle, options.max_kernel_objects).await?;
        let routes = get_routes(&handle, options.max_kernel_objects).await?;
        let mut journal = match previous {
            Some(journal)
                if !journal.rules.is_empty()
                    || !journal.routes.is_empty()
                    || journal.anchor.is_some() =>
            {
                journal
            }
            _ => allocate(&rules, &routes, options.max_pools)?,
        };
        if journal.pool_limit != options.max_pools {
            return Err(Error::Options);
        }
        verify_rules(&journal, &rules)?;
        // A previous process's nonpersistent TUN is never adopted. Any live
        // positive route in its private table is ambiguous and blocks recovery.
        if routes
            .iter()
            .any(|route| route_table(route) == journal.table)
        {
            return Err(Error::Conflict);
        }
        journal.routes.clear();
        journal.tun = None;
        let addresses = get_addresses(&handle, options.max_kernel_objects).await?;
        if addresses
            .iter()
            .any(|address| capture::address_conflicts(&journal.pools, None, address))
            || routes
                .iter()
                .any(|route| capture::route_conflicts(&journal.pools, None, route))
        {
            return Err(Error::Conflict);
        }
        let foreign: Vec<_> = rules
            .into_iter()
            .filter(|rule| {
                !journal
                    .rules
                    .iter()
                    .any(|record| record.observed == encode_rule(rule.clone()))
            })
            .collect();
        let selectors = plan::selectors(&foreign)?;
        journal.mark.transform(0, &selectors)?;
        let mut baseline: Vec<_> = foreign.into_iter().map(encode_rule).collect();
        baseline.sort();
        let bootstrap_sockets = (options.max_sockets / 8).clamp(1, 16);
        let shared = Arc::new(Shared {
            resources: super::protection::Resources::new(options.max_sockets * 4),
            mark: journal.mark,
            selectors,
            conflict,
            network_changes: tokio::sync::watch::channel(0).0,
            tasks: tasks.clone(),
            sockets: Arc::new(Semaphore::new(bootstrap_sockets)),
            data_sockets: Arc::new(Semaphore::new(options.max_sockets - bootstrap_sockets)),
            attempts: Arc::new(Semaphore::new(options.max_sockets)),
            resolutions: Arc::new(Semaphore::new(options.max_sockets)),
            execution: RwLock::new(None),
        });
        // Permissions and readback are real even before the first connection.
        for socket in [
            tokio::net::TcpSocket::new_v4()?,
            tokio::net::TcpSocket::new_v6()?,
        ] {
            shared.prepare(&socket2::SockRef::from(&socket))?;
        }
        if let Some(anchor) = &journal.anchor {
            anchor::verify(&handle, anchor, options.max_kernel_objects).await?;
        }
        let expected = Arc::new(Mutex::new(Expected {
            anchor: journal.anchor.clone(),
            pools: journal.pools.clone(),
            ..Expected::default()
        }));
        let watched = shared.clone();
        let events = expected.clone();
        let table = journal.table;
        tasks.spawn(async move {
            monitor::run(monitor_socket, watched, events, table).await;
        })?;
        store.persist(&journal).await?;
        shared.check()?;
        Ok(Self {
            handle,
            store,
            journal,
            shared,
            expected,
            tun: None,
            options,
            baseline,
            dns_cancel: CancellationToken::new(),
        })
    }

    async fn ready(&mut self) -> Result<()> {
        self.store.settle().await?;
        self.shared.check()?;
        if self.journal.pending.is_some() {
            return Err(Error::Journal);
        }
        Ok(())
    }

    async fn verify(&self) -> Result<()> {
        if let Some(anchor) = &self.journal.anchor {
            anchor::verify(&self.handle, anchor, self.options.max_kernel_objects).await?;
        }
        let rules = get_rules(&self.handle, self.options.max_kernel_objects).await?;
        verify_rules(&self.journal, &rules)?;
        let mut foreign: Vec<_> = rules
            .into_iter()
            .filter(|rule| {
                !self
                    .journal
                    .rules
                    .iter()
                    .any(|record| record.observed == encode_rule(rule.clone()))
            })
            .map(encode_rule)
            .collect();
        foreign.sort();
        if foreign != self.baseline {
            return Err(Error::Conflict);
        }
        let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
        self.verify_capture_routes(&self.journal.pools, &routes)
            .await?;
        let actual: Vec<_> = routes
            .into_iter()
            .filter(|route| route_table(route) == self.journal.table)
            .map(encode_route)
            .collect();
        if actual.len() != self.journal.routes.len()
            || self.journal.routes.iter().any(|record| {
                actual
                    .iter()
                    .filter(|bytes| **bytes == record.observed)
                    .count()
                    != 1
            })
        {
            return Err(Error::Conflict);
        }
        Ok(())
    }

    async fn verify_capture(&self, pools: &[ipnet::IpNet]) -> Result<()> {
        let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
        self.verify_capture_routes(pools, &routes).await
    }

    async fn verify_capture_routes(
        &self,
        pools: &[ipnet::IpNet],
        routes: &[RouteMessage],
    ) -> Result<()> {
        let addresses = get_addresses(&self.handle, self.options.max_kernel_objects).await?;
        let mut assigned = Vec::new();
        let owned = if let Some(tun) = &self.tun {
            let record = self.journal.tun.as_ref().ok_or(Error::Conflict)?;
            let index = tun.if_index()?;
            if index == 0 || index != record.ifindex || tun.name() != record.name {
                return Err(Error::Conflict);
            }
            assigned.extend(
                [
                    record.addresses.ipv4.map(IpAddr::V4),
                    record.addresses.ipv6.map(IpAddr::V6),
                ]
                .into_iter()
                .flatten(),
            );
            Some(capture::Owned {
                index,
                addresses: &assigned,
            })
        } else {
            None
        };
        if routes
            .iter()
            .any(|route| capture::route_conflicts(pools, owned, route))
            || addresses
                .iter()
                .any(|address| capture::address_conflicts(pools, owned, address))
        {
            return Err(Error::Conflict);
        }
        self.shared.check()
    }

    pub async fn install(&mut self, snapshot: ExecutionSnapshot) -> Result<()> {
        self.ready().await?;
        let cfg = snapshot.configuration().map_err(|_| Error::Expired)?;
        let binding = (cfg.network_id, cfg.node_id);
        if self
            .journal
            .binding
            .is_some_and(|previous| previous != binding)
        {
            return Err(Error::Target);
        }
        if cfg.exit_enabled
            || cfg.permits.values().any(|permit| {
                !matches!(
                    permit.target,
                    meshway_configuration::execution::BusinessTarget::Mesh { .. }
                )
            })
        {
            return Err(Error::Target);
        }
        for candidate in cfg.dialers.values() {
            validate_dial(&candidate.parameters)?;
        }
        for listener in cfg.listeners.values() {
            if listener_enabled(listener.declaration()) {
                validate_listener(listener.declaration())?;
            }
        }
        let pools = plan::pools(
            &cfg.mesh_address_pools,
            &cfg.addresses,
            self.options.max_pools,
        )?;
        if self.journal.anchor.as_ref().is_some_and(|anchor| {
            pools
                .iter()
                .any(|pool| pool.contains(&IpAddr::V4(anchor.identity.address)))
        }) {
            return Err(Error::Conflict);
        }
        let addresses = cfg.addresses.clone();
        self.verify_capture(&pools).await?;
        if self.retain_tun(&snapshot, &pools, &addresses).await? {
            return Ok(());
        }
        self.verify().await?;
        self.journal.binding = Some(binding);
        self.store.persist(&self.journal).await?;
        // Fence the old packet/carrier scope, then protect the complete new
        // pool set before removing any old positive routes or the owned TUN.
        // Retired pools keep their negative rules until explicit down.
        self.shared.suspend()?;
        self.retain_negative(&pools).await?;
        self.suspend().await?;
        let outcome = self.activate(&snapshot, &pools, &addresses).await;
        if outcome.is_err() {
            if self.journal.pending.is_none() {
                // Known creations can be rolled back without turning ordinary
                // lease expiry during installation into a protection conflict.
                if let Err(error) = self.suspend().await {
                    self.shared.conflict.cancel();
                    return Err(error);
                }
            } else {
                self.shared.suspend()?;
                self.shared.conflict.cancel();
            }
            // Descriptor ownership makes dropping this new device safe even if
            // an interrupted netlink operation has an uncertain outcome.
            self.drop_owned_tun();
        }
        outcome
    }

    /// A Control-issued direct publication successor may retain the host IP
    /// interface even though Control already invalidated the predecessor's I/O.
    /// The local socket cancellation token is not the Control lineage proof.
    async fn retain_tun(
        &self,
        snapshot: &ExecutionSnapshot,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<bool> {
        let Some(tun) = &self.tun else {
            return Ok(false);
        };
        let previous = self
            .shared
            .execution
            .read()
            .map_err(|_| Error::Worker)?
            .as_ref()
            .map(|(snapshot, _)| snapshot.clone());
        let Some(previous) = previous else {
            return Ok(false);
        };
        let Ok(configuration) = snapshot.continuation_from(&previous) else {
            return Ok(false);
        };
        let old_pools = plan::pools(
            &configuration.mesh_address_pools,
            &configuration.addresses,
            self.options.max_pools,
        )?;
        if !same_pools(&old_pools, pools) {
            return Ok(false);
        }

        self.verify().await?;
        self.verify_tun(tun, pools, addresses).await?;
        self.quiesce().await?;
        self.shared.check()?;
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        let mut active = self.shared.execution.write().map_err(|_| Error::Worker)?;
        let Some((installed, token)) = active.as_mut() else {
            return Err(Error::Expired);
        };
        if !installed.same_execution(&previous) {
            return Err(Error::Expired);
        }
        // Netlink reads await: recheck both original deadlines and the direct
        // owner handoff immediately before publishing the new execution guard.
        if snapshot.continuation_from(installed).is_err() {
            return Ok(false);
        }
        if !token.is_cancelled() {
            self.shared.resources.handoff(snapshot)?;
            *installed = snapshot.clone();
        } else {
            token.cancel();
            *active = Some((snapshot.clone(), CancellationToken::new()));
        }
        Ok(true)
    }

    async fn verify_tun(
        &self,
        tun: &Tun,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<()> {
        let record = self.journal.tun.as_ref().ok_or(Error::Conflict)?;
        let index = tun.if_index()?;
        if record.ifindex != index
            || record.name != tun.name()
            || record.mtu != self.options.mtu
            || tun.mtu() != self.options.mtu
            || record.addresses != *addresses
            || !live_routes_match(&self.journal, index, pools, addresses)
        {
            return Err(Error::Conflict);
        }
        let links = collect(
            self.handle.link().get().match_index(index).execute(),
            self.options.max_kernel_objects,
        )
        .await?;
        if links.len() != 1 {
            return Err(Error::Conflict);
        }
        let link = &links[0];
        use rtnetlink::packet_route::link::{LinkAttribute, LinkFlags};
        if link.header.index != index
            || !link.header.flags.contains(LinkFlags::Up)
            || !link
                .attributes
                .iter()
                .any(|a| matches!(a, LinkAttribute::IfName(name) if name == tun.name()))
            || !link
                .attributes
                .iter()
                .any(|a| matches!(a, LinkAttribute::Mtu(mtu) if *mtu as usize == self.options.mtu))
        {
            return Err(Error::Conflict);
        }
        let actual = get_addresses(&self.handle, self.options.max_kernel_objects).await?;
        // Verification is a single bounded dump, not a new readiness grace
        // period. The Control handoff's original deadlines remain authoritative.
        for wanted in [
            addresses.ipv4.map(IpAddr::V4),
            addresses.ipv6.map(IpAddr::V6),
        ]
        .into_iter()
        .flatten()
        {
            let matches: Vec<_> = actual.iter().filter(|address| address.header.index == index
                && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(ip) | AddressAttribute::Local(ip) if *ip == wanted))).collect();
            if matches.len() != 1 {
                return Err(Error::Conflict);
            }
            let address = matches[0];
            let flags = address
                .attributes
                .iter()
                .find_map(|a| {
                    if let AddressAttribute::Flags(flags) = a {
                        Some(flags.bits())
                    } else {
                        None
                    }
                })
                .unwrap_or(u32::from(address.header.flags.bits()));
            if flags & 0xc8 != 0x80
                || address.header.prefix_len != if wanted.is_ipv4() { 32 } else { 128 }
            {
                return Err(Error::Conflict);
            }
        }
        Ok(())
    }

    /// Only a negative boundary. This creates no TUN, socket permit, or grant.
    async fn retain_negative(&mut self, pools: &[ipnet::IpNet]) -> Result<()> {
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
        snapshot: &ExecutionSnapshot,
        pools: &[ipnet::IpNet],
        addresses: &meshway_configuration::NodeAddresses,
    ) -> Result<()> {
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
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
            snapshot.ensure_current().map_err(|_| Error::Expired)?;
            self.add_route(route).await?;
        }
        self.quiesce().await?;
        self.expected.lock().map_err(|_| Error::Worker)?.changing = false;
        self.verify().await?;
        self.quiesce().await?;
        snapshot.ensure_current().map_err(|_| Error::Expired)?;
        self.shared.check()?;
        *self.shared.execution.write().map_err(|_| Error::Worker)? =
            Some((snapshot.clone(), CancellationToken::new()));
        Ok(())
    }

    async fn address_ready(
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
                    let flags = address.attributes.iter().find_map(|a| if let AddressAttribute::Flags(flags) = a { Some(flags.bits()) } else { None }).unwrap_or(u32::from(address.header.flags.bits()));
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

    fn drop_owned_tun(&mut self) {
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
            self.expect(false, true, record.observed.clone())?;
            execute(
                self.handle
                    .route()
                    .del(decode_route(&record.observed)?)
                    .execute(),
            )
            .await?;
            let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
            if routes
                .into_iter()
                .any(|route| encode_route(route) == record.observed)
            {
                return Err(Error::Conflict);
            }
            self.journal.routes.pop();
            self.journal.pending = None;
            self.store.persist(&self.journal).await?;
        }
        self.quiesce().await
    }

    pub async fn down(&mut self) -> Result<()> {
        self.suspend().await?;
        self.ready().await?;
        // Bypass and end are removed last; protected bootstrap continues through
        // the original RPDB throughout explicit cleanup.
        let mut records = self.journal.rules.clone();
        records.sort_by_key(|record| match record.spec.kind {
            RuleKind::Lookup { .. } => 0,
            RuleKind::Sink { .. } => 1,
            RuleKind::Bypass { .. } => 2,
            RuleKind::End => 3,
        });
        for record in records {
            self.verify().await?;
            self.journal.pending = Some(Pending::DeleteRule {
                record: record.clone(),
            });
            self.store.persist(&self.journal).await?;
            self.expect(true, true, record.observed.clone())?;
            execute(
                self.handle
                    .rule()
                    .del(decode_rule(&record.observed)?)
                    .execute(),
            )
            .await?;
            let rules = get_rules(&self.handle, self.options.max_kernel_objects).await?;
            if rules.into_iter().any(|rule| {
                rule.header.family == record.spec.message().header.family
                    && priority(&rule) == record.spec.priority
            }) {
                return Err(Error::Conflict);
            }
            self.journal
                .rules
                .retain(|existing| existing.spec != record.spec);
            self.journal.pending = None;
            self.store.persist(&self.journal).await?;
        }
        self.journal.pools.clear();
        self.journal.binding = None;
        self.store.persist(&self.journal).await?;
        self.quiesce().await?;
        self.verify().await?;
        self.expected
            .lock()
            .map_err(|_| Error::Worker)?
            .pools
            .clear();
        Ok(())
    }

    pub async fn settle(&mut self) -> Result<()> {
        self.store.settle().await
    }

    async fn add_rule(&mut self, spec: RuleSpec) -> Result<()> {
        self.ready().await?;
        self.verify().await?;
        if self.journal.rules.iter().any(|record| record.spec == spec) {
            return Ok(());
        }
        self.journal.pending = Some(Pending::AddRule { spec: spec.clone() });
        self.store.persist(&self.journal).await?;
        let message = spec.message();
        let expected = encode_rule(message.clone());
        self.expect(true, false, expected.clone())?;
        let mut request = self.handle.rule().add();
        *request.message_mut() = message;
        execute(request.execute()).await?;
        let rules = get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let matches: Vec<_> = rules
            .into_iter()
            .filter(|rule| {
                rule.header.family == spec.message().header.family
                    && priority(rule) == spec.priority
            })
            .collect();
        if matches.len() != 1 || encode_rule(matches[0].clone()) != expected {
            return Err(Error::Conflict);
        }
        self.journal.rules.push(RuleRecord {
            spec,
            observed: expected,
        });
        self.journal.pending = None;
        self.store.persist(&self.journal).await
    }

    async fn add_route(&mut self, spec: RouteSpec) -> Result<()> {
        self.ready().await?;
        self.verify().await?;
        self.journal.pending = Some(Pending::AddRoute { spec: spec.clone() });
        self.store.persist(&self.journal).await?;
        let message = spec.message()?;
        let expected = encode_route(message.clone());
        self.expect(false, false, expected.clone())?;
        execute(self.handle.route().add(message).execute()).await?;
        let routes = get_routes(&self.handle, self.options.max_kernel_objects).await?;
        if routes
            .into_iter()
            .filter(|route| encode_route(route.clone()) == expected)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
        self.journal.routes.push(RouteRecord {
            spec,
            observed: expected,
        });
        self.journal.pending = None;
        self.store.persist(&self.journal).await
    }

    fn expect(&self, rule: bool, deleted: bool, bytes: Vec<u8>) -> Result<()> {
        self.expected.lock().map_err(|_| Error::Worker)?.add(Event {
            rule,
            deleted,
            bytes,
        })
    }

    async fn quiesce(&self) -> Result<()> {
        let (ticket, wake) = {
            let mut state = self.expected.lock().map_err(|_| Error::Worker)?;
            state.requested_drain = state.requested_drain.checked_add(1).ok_or(Error::Worker)?;
            (state.requested_drain, state.wake.clone())
        };
        // TUN creation can have no positive-route notification (for example a
        // relay with no assigned IPs). Drain the monitor explicitly before
        // leaving its own-device creation phase, even when events is empty.
        wake.notify_one();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                self.shared.check()?;
                let drained = {
                    let state = self.expected.lock().map_err(|_| Error::Worker)?;
                    state.events.is_empty() && state.drained >= ticket
                };
                if drained {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .map_err(|_| Error::Conflict)?
    }
}

fn verify_rules(journal: &Journal, rules: &[RuleMessage]) -> Result<()> {
    for record in &journal.rules {
        let matching: Vec<_> = rules
            .iter()
            .filter(|rule| {
                rule.header.family == record.spec.message().header.family
                    && priority(rule) == record.spec.priority
            })
            .collect();
        if matching.len() != 1 || encode_rule(matching[0].clone()) != record.observed {
            return Err(Error::Conflict);
        }
    }
    for rule in rules {
        if ((journal.start..=journal.end()).contains(&priority(rule))
            || rule_table(rule) == journal.table)
            && !journal
                .rules
                .iter()
                .any(|record| record.observed == encode_rule(rule.clone()))
        {
            return Err(Error::Conflict);
        }
        if rule.attributes.iter().any(|a| matches!(a, RuleAttribute::Goto(target) if (journal.start..=journal.end()).contains(target)))
            && !journal.rules.iter().any(|record| record.observed == encode_rule(rule.clone())) { return Err(Error::Conflict); }
    }
    Ok(())
}

fn same_pools(left: &[ipnet::IpNet], right: &[ipnet::IpNet]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

fn live_routes_match(
    journal: &Journal,
    index: u32,
    pools: &[ipnet::IpNet],
    addresses: &meshway_configuration::NodeAddresses,
) -> bool {
    let expected = plan::positive_routes(pools, addresses, journal.table, index);
    journal.routes.len() == expected.len()
        && expected.iter().all(|route| {
            journal
                .routes
                .iter()
                .filter(|record| record.spec == *route)
                .count()
                == 1
        })
}

fn allocate(rules: &[RuleMessage], routes: &[RouteMessage], pool_limit: usize) -> Result<Journal> {
    let selectors = plan::selectors(rules)?;
    // Only the ordinary local lookup may precede our capture block.
    if rules.iter().filter(|r| priority(r) == 0).any(|rule| {
        rule_table(rule) != 255
            || rule.header.action != rtnetlink::packet_route::rule::RuleAction::ToTable
            || rule.header.src_len != 0
            || rule.header.dst_len != 0
            || rule.header.tos != 0
            || !rule.header.flags.is_empty()
            || rule.attributes.iter().any(|a| {
                !matches!(
                    a,
                    RuleAttribute::Table(_)
                        | RuleAttribute::Priority(_)
                        | RuleAttribute::Protocol(_)
                        | RuleAttribute::SuppressPrefixLen(u32::MAX)
                        | RuleAttribute::SuppressIfGroup(u32::MAX)
                )
            })
    }) {
        return Err(Error::Conflict);
    }
    for family in [AddressFamily::Inet, AddressFamily::Inet6] {
        if rules
            .iter()
            .filter(|r| r.header.family == family && priority(r) == 0)
            .count()
            != 1
        {
            return Err(Error::Conflict);
        }
    }
    let first = rules
        .iter()
        .map(priority)
        .filter(|p| *p > 0)
        .min()
        .unwrap_or(32766)
        .min(32766);
    let width = pool_limit as u32 * 2 + 2;
    if first <= width {
        return Err(Error::Conflict);
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
    let seed = u32::from_ne_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
    let start = (0..1024)
        .find_map(|attempt| {
            let start = 1 + seed.wrapping_add(attempt) % (first - width);
            let end = start + width - 1;
            (!rules.iter().any(|rule| {
                rule.attributes.iter().any(
                    |a| matches!(a, RuleAttribute::Goto(target) if (start..=end).contains(target)),
                )
            }))
            .then_some(start)
        })
        .ok_or(Error::Conflict)?;
    let table = (0..1024)
        .map(|offset| 10000 + seed.wrapping_add(offset) % (u32::MAX - 20000))
        .find(|table| {
            !rules.iter().any(|rule| rule_table(rule) == *table)
                && !routes.iter().any(|route| route_table(route) == *table)
        })
        .ok_or(Error::Conflict)?;
    let mark = (0..32)
        .map(|shift| {
            let bit = 1u32 << ((shift + seed % 32) % 32);
            Mark {
                value: bit,
                mask: bit,
            }
        })
        .find(|mark| mark.transform(0, &selectors).is_ok())
        .ok_or(Error::Conflict)?;
    let (boot, namespace_device, namespace_inode) = Journal::identity()?;
    let installation = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(Journal {
        version: 1,
        boot,
        namespace_device,
        namespace_inode,
        installation,
        binding: None,
        mark,
        table,
        start,
        pool_limit,
        pools: Vec::new(),
        rules: Vec::new(),
        routes: Vec::new(),
        tun: None,
        anchor: None,
        pending: None,
    })
}

async fn collect<S>(mut stream: S, limit: usize) -> Result<Vec<S::Ok>>
where
    S: TryStream<Error = rtnetlink::Error> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut result = Vec::new();
        while let Some(message) = stream.try_next().await.map_err(|_| Error::Netlink)? {
            if result.len() == limit {
                return Err(Error::Capacity);
            }
            result.push(message);
        }
        Ok(result)
    })
    .await
    .map_err(|_| Error::Netlink)?
}

async fn execute(
    operation: impl std::future::Future<Output = std::result::Result<(), rtnetlink::Error>>,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .map_err(|_| Error::Netlink)?
        .map_err(|_| Error::Netlink)
}

async fn get_rules(handle: &Handle, limit: usize) -> Result<Vec<RuleMessage>> {
    let mut rules = collect(handle.rule().get(IpVersion::V4).execute(), limit).await?;
    rules.extend(
        collect(
            handle.rule().get(IpVersion::V6).execute(),
            limit.saturating_sub(rules.len()),
        )
        .await?,
    );
    Ok(rules)
}

async fn get_routes(handle: &Handle, limit: usize) -> Result<Vec<RouteMessage>> {
    let mut routes = Vec::new();
    for family in [AddressFamily::Inet, AddressFamily::Inet6] {
        let mut message = RouteMessage::default();
        message.header.address_family = family;
        routes.extend(
            collect(
                handle.route().get(message).execute(),
                limit.saturating_sub(routes.len()),
            )
            .await?,
        );
    }
    Ok(routes)
}

async fn get_addresses(handle: &Handle, limit: usize) -> Result<Vec<AddressMessage>> {
    collect(handle.address().get().execute(), limit).await
}

#[cfg(test)]
mod tests;
