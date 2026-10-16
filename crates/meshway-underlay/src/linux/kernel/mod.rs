mod anchor;
mod capture;
pub(crate) mod egress;
pub(crate) mod exit;
mod journal;
mod monitor;
mod namespace;
pub(super) mod network;
mod plan;
mod rpdb;
mod tool;
mod verification;

use futures_util::{TryStream, TryStreamExt};
use meshway_engine::Desired;
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
use crate::{Error, Options, Result, target::View};
use journal::{Journal, Pending, RouteRecord, RuleRecord, Store, TunRecord};
use monitor::Expected;
use plan::{
    RouteSpec, RuleKind, RuleSpec, encode_route, encode_rule, priority, route_table, rule_table,
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
    pub(super) exit: Option<exit::owner::Owner>,
    probe: Option<exit::ExitProbe>,
    egress: Option<egress::Owner>,
    /// The MSS clamp last handed to the capture owner.
    clamp: Option<egress::mss::Clamp>,
    /// When owned capture objects were last reinstalled ([`Self::repair`]).
    repaired: std::collections::VecDeque<std::time::Instant>,
}

/// Repairs of removed capture objects allowed within [`REPAIR_WINDOW`].
const MAX_REPAIRS: usize = 3;
const REPAIR_WINDOW: Duration = Duration::from_secs(60);

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
            .chain(self.exit.iter().map(|owner| owner.interface_name.clone()))
            .collect()
    }

    pub async fn open(directory: Arc<Directory>, options: Options, tasks: Tasks) -> Result<Self> {
        // Subscribe before the baseline dump so rule changes cannot disappear
        // between discovery and the monitor's start.
        let (connection, handle, unsolicited) = rtnetlink::new_connection()?;
        drop(unsolicited); // request socket has no multicast subscriptions
        let conflict = CancellationToken::new();
        let failed = conflict.clone();
        tasks.spawn(async move {
            connection.await;
            failed.cancel();
        })?;
        exit::recovery::recover(&handle, directory.clone(), options.max_kernel_objects).await?;
        egress::recover(
            &handle,
            directory.clone(),
            options.max_kernel_objects,
            false,
        )
        .await?;
        let monitor_socket = monitor::open()?;
        // Observation (change fingerprints, route lookups) has its own
        // request socket: it never queues behind or interleaves with the
        // owner's mutations and their readback dumps.
        let (observer_connection, observer, unsolicited) = rtnetlink::new_connection()?;
        drop(unsolicited);
        let observer_failed = conflict.clone();
        tasks.spawn(async move {
            observer_connection.await;
            observer_failed.cancel();
        })?;
        let (mut store, mut previous) = Store::open(directory).await?;
        let rules = get_rules(&handle, options.max_kernel_objects).await?;
        let routes = get_routes(&handle, options.max_kernel_objects).await?;
        // The same boot and nsfs inode but none of the journal's objects: a
        // new namespace reused the inode (container stop/start). Nothing here
        // is ours to adopt or clean up; a partial set stays a conflict below.
        if let Some(journal) = previous
            .as_ref()
            .filter(|journal| !journal.rules.is_empty() || journal.anchor.is_some())
        {
            let observed: Vec<_> = rules.iter().cloned().map(encode_rule).collect();
            let aliases = link_aliases(&handle, options.max_kernel_objects).await?;
            if !journal.resident(&observed, &aliases) {
                store.archive_foreign(journal).await?;
                previous = None;
            }
        }
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
            network: network::Network::with_routes(observer.clone()),
            tasks: tasks.clone(),
            sockets: Arc::new(Semaphore::new(bootstrap_sockets)),
            data_sockets: Arc::new(Semaphore::new(options.max_sockets - bootstrap_sockets)),
            attempts: Arc::new(Semaphore::new(options.max_sockets)),
            resolutions: Arc::new(Semaphore::new(options.max_sockets)),
            execution: RwLock::new(None),
            changes: tokio::sync::watch::channel(None).0,
        });
        // Permissions and readback are real even before the first connection.
        for socket in [
            tokio::net::TcpSocket::new_v4()?,
            tokio::net::TcpSocket::new_v6()?,
        ] {
            shared.prepare(&socket2::SockRef::from(&socket))?;
        }
        if let Some(anchor) = &journal.anchor {
            // Recovery above left no route in a source-capture table.
            anchor::verify(&handle, anchor, None, options.max_kernel_objects).await?;
        }
        let expected = Arc::new(Mutex::new(Expected {
            anchor: journal.anchor.clone(),
            pools: journal.pools.clone(),
            own_rules: journal
                .rules
                .iter()
                .map(|record| record.observed.clone())
                .collect(),
            ..Expected::default()
        }));
        let watched = shared.clone();
        let events = expected.clone();
        let table = journal.table;
        tasks.spawn(async move {
            monitor::run(monitor_socket, watched, events, table).await;
        })?;
        tasks.spawn(network::classify(
            shared.clone(),
            observer,
            expected.clone(),
            options.max_kernel_objects,
        ))?;
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
            exit: None,
            probe: None,
            egress: None,
            clamp: None,
            repaired: std::collections::VecDeque::new(),
        })
    }

    /// The monitor's view of the capture block's own rules follows the journal.
    fn publish_own_rules(&self) -> Result<()> {
        self.expected.lock().map_err(|_| Error::Worker)?.own_rules = self
            .journal
            .rules
            .iter()
            .map(|record| record.observed.clone())
            .collect();
        Ok(())
    }

    /// Reinstall journal rules and capture-table routes that something else
    /// removed (a network manager cleaning up rules it does not know), in
    /// the order the block needs them: end and bypass before the pool rules,
    /// each pool's sink before its lookup. A slot another owner holds now is
    /// a conflict; so is a fourth repair within a minute.
    pub async fn repair(&mut self) -> Result<()> {
        let now = std::time::Instant::now();
        while self
            .repaired
            .front()
            .is_some_and(|at| now.duration_since(*at) >= REPAIR_WINDOW)
        {
            self.repaired.pop_front();
        }
        if self.repaired.len() >= MAX_REPAIRS {
            tracing::warn!("Owned capture objects keep being removed; ownership ends");
            self.shared.conflict.cancel();
            return Err(Error::Conflict);
        }
        self.repaired.push_back(now);
        self.ready().await?;
        let present: Vec<_> = get_rules(&self.handle, self.options.max_kernel_objects)
            .await?
            .into_iter()
            .map(encode_rule)
            .collect();
        let mut missing: Vec<_> = self
            .journal
            .rules
            .iter()
            .filter(|record| !present.contains(&record.observed))
            .cloned()
            .collect();
        missing.sort_by_key(|record| match record.spec.kind {
            RuleKind::End => 0,
            RuleKind::Bypass { .. } => 1,
            RuleKind::Sink { .. } => 2,
            RuleKind::Lookup { .. } => 3,
        });
        let routes: Vec<_> = get_routes(&self.handle, self.options.max_kernel_objects)
            .await?
            .into_iter()
            .map(encode_route)
            .collect();
        let lost: Vec<_> = self
            .journal
            .routes
            .iter()
            .filter(|record| !routes.contains(&record.observed))
            .cloned()
            .collect();
        tracing::warn!(
            rules = missing.len(),
            routes = lost.len(),
            "Reinstalling owned capture objects removed outside their owner"
        );
        let result = async {
            for record in &missing {
                self.objects().add_rule(record.spec.message()).await?;
            }
            for record in &lost {
                self.objects().add_route(record.spec.message()?).await?;
            }
            self.quiesce().await?;
            self.verify().await
        }
        .await;
        if result.is_err() {
            self.shared.conflict.cancel();
        }
        result
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
            let capture = self
                .expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table;
            anchor::verify(
                &self.handle,
                anchor,
                capture,
                self.options.max_kernel_objects,
            )
            .await?;
        }
        let mut rules = get_rules(&self.handle, self.options.max_kernel_objects).await?;
        let exit_rules = {
            let expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected
                .exit_rules
                .iter()
                .chain(&expected.egress_rules)
                .cloned()
                .collect::<Vec<_>>()
        };
        // An exit rule whose input TUN is gone carries the kernel's
        // IIF_DETACHED state flag; it is still that owner's own rule.
        rules.retain(|rule| {
            let mut attached = rule.clone();
            attached
                .header
                .flags
                .remove(rtnetlink::packet_route::rule::RuleFlags::IifDetached);
            !exit_rules.contains(&encode_rule(rule.clone()))
                && !exit_rules.contains(&encode_rule(attached))
        });
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
        // Removal and repair proceed while an external address overlaps a
        // pool; only installation refuses it.
        self.verify_capture_routes(&self.journal.pools, &routes, true)
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
        self.verify_capture_routes(pools, &routes, false).await
    }

    async fn verify_capture_routes(
        &self,
        pools: &[ipnet::IpNet],
        routes: &[RouteMessage],
        overlap: bool,
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
        let expected = self.expected.lock().map_err(|_| Error::Worker)?;
        let tolerated = |route: &RouteMessage| overlap && monitor::overlap_route(&expected, route);
        if routes
            .iter()
            .any(|route| capture::route_conflicts(pools, owned, route) && !tolerated(route))
            || addresses.iter().any(|address| {
                capture::address_conflicts(pools, owned, address)
                    && !(overlap && expected.overlapping(address))
            })
        {
            return Err(Error::Conflict);
        }
        drop(expected);
        self.shared.check()
    }

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
            super::protection::Installed::new(desired.clone(), CancellationToken::new()),
        );
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

    pub async fn down(&mut self) -> Result<()> {
        self.suspend().await?;
        if let Some(mut owner) = self.egress.take() {
            self.clamp = None;
            owner.shutdown(true).await?;
            self.quiesce().await?;
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table = None;
        } else {
            egress::recover(
                &self.handle,
                self.store.directory(),
                self.options.max_kernel_objects,
                true,
            )
            .await?;
        }
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
            let objects = self.objects();
            let observed = objects.owned_rule(&record.observed, None).await?;
            objects.remove_rule(&record.observed, observed).await?;
            self.journal
                .rules
                .retain(|existing| existing.spec != record.spec);
            self.publish_own_rules()?;
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
    /// Clamp the TCP MSS of SYNs crossing the Mesh TUN (and the exit TUN) to
    /// `limits`. Only a change reaches nft; without a capture owner or TUN
    /// nothing is clamped.
    pub fn clamp(&mut self, limits: meshway_tun::PathLimits) -> Result<()> {
        let Some(owner) = &self.egress else {
            return Ok(());
        };
        let clamp = {
            let expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected
                .tun
                .as_ref()
                .map(|(_, name, mtu, _)| egress::mss::Clamp {
                    mesh: name.clone(),
                    exit: self.exit.as_ref().map(|exit| exit.interface_name.clone()),
                    pools: expected.pools.clone(),
                    mtu: *mtu,
                    limits,
                })
        };
        // An unchanged clamp is done only once its replacement succeeded:
        // a failed nft transaction is submitted again after its backoff.
        if clamp == self.clamp && !owner.capture.clamp_retry_due() {
            return Ok(());
        }
        owner.capture.clamp(clamp.clone())?;
        self.clamp = clamp;
        Ok(())
    }
    /// Activate the staged capture plan. An already active, unchanged plan is
    /// a resource left as is across a continuation.
    async fn resume_egress(&self) -> Result<()> {
        if let Some(owner) = &self.egress
            && !owner.capture.is_active()
        {
            owner.capture.switch(owner.capture.plan()).await?;
        }
        Ok(())
    }
    pub async fn finish_egress(&mut self) -> Result<()> {
        if let Some(mut owner) = self.egress.take() {
            self.clamp = None;
            owner.shutdown(false).await?;
        }
        Ok(())
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
        self.objects().add_rule(message).await?;
        self.journal.rules.push(RuleRecord {
            spec,
            observed: expected,
        });
        self.publish_own_rules()?;
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
        self.objects().add_route(message).await?;
        self.journal.routes.push(RouteRecord {
            spec,
            observed: expected,
        });
        self.journal.pending = None;
        self.store.persist(&self.journal).await
    }

    fn objects(&self) -> rpdb::Objects<'_> {
        rpdb::Objects {
            handle: &self.handle,
            expected: &self.expected,
            limit: self.options.max_kernel_objects,
            monitored: true,
        }
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

/// Interface aliases of this namespace.
async fn link_aliases(handle: &Handle, limit: usize) -> Result<Vec<String>> {
    Ok(collect(handle.link().get().execute(), limit)
        .await?
        .into_iter()
        .flat_map(|link| link.attributes)
        .filter_map(|attribute| match attribute {
            rtnetlink::packet_route::link::LinkAttribute::IfAlias(alias) => Some(alias),
            _ => None,
        })
        .collect())
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

/// The installed `previous` content as a view, when `next` is new content
/// for the same Node of the same network. Comparison data only.
fn continuation<'a>(next: &Desired, previous: &'a Desired) -> Option<View<'a>> {
    (!next.same(previous)
        && next.netmap().network_id == previous.netmap().network_id
        && next.netmap().node_id == previous.netmap().node_id
        && next.local().self_keys == previous.local().self_keys)
        .then(|| View::of(previous))
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
    let width = pool_limit as u32 * 2 + 2 + 32;
    let reserved = exit::routing::RESERVED;
    if first <= width + reserved {
        return Err(Error::Conflict);
    }
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| Error::Worker)?;
    let seed = u32::from_ne_bytes(random[..4].try_into().map_err(|_| Error::Worker)?);
    let start = (0..1024)
        .find_map(|attempt| {
            let start = 1 + reserved + seed.wrapping_add(attempt) % (first - width - reserved);
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
    let namespace = namespace::Identity::current()?;
    let installation = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(Journal {
        version: 1,
        boot: namespace.boot,
        namespace_device: namespace.device,
        namespace_inode: namespace.inode,
        namespace_cookie: namespace.cookie,
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

/// Route objects of every table. A cached exception the kernel derives from
/// a route (`RTM_F_CLONED`: a PMTU learnt from fragmentation needed, a
/// redirect) is not a route: it lives and dies with its parent, nobody adds
/// it, and an ICMP error reaching a captured flow creates one in an owned
/// table. Ownership checks never see it.
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
            .await?
            .into_iter()
            .filter(|route| {
                !route
                    .header
                    .flags
                    .contains(rtnetlink::packet_route::route::RouteFlags::Cloned)
            }),
        );
    }
    Ok(routes)
}

async fn get_addresses(handle: &Handle, limit: usize) -> Result<Vec<AddressMessage>> {
    collect(handle.address().get().execute(), limit).await
}

#[cfg(test)]
mod tests;
