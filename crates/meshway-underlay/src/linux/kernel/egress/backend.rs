use super::super::rtnl::Handle;
use super::super::{
    exit::objects::{self, Receipt},
    monitor::Expected,
    plan::{PROTOCOL, encode_route, encode_rule, priority, route_table},
    tool::command::Commands,
    verification::{Reread, Verification},
};
use super::{
    plan::{self, Ranges},
    storage::{Device, Gate, Ledger, Store},
};
use crate::{Error, Result};
use meshway_configuration::execution::EgressCapturePlan;
use rtnetlink::{
    RouteMessageBuilder,
    packet_route::{
        AddressFamily,
        route::{RouteMessage, RouteScope, RouteType},
        rule::{RuleAction, RuleAttribute, RuleMessage},
    },
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{Arc, Mutex},
};

/// Route mutations between two waits for the kernel monitor. The socket
/// buffer is charged per notification buffer (about 1 KiB each), so the
/// default 208 KiB holds about 200: 64 leave a wide margin.
const ROUTE_BATCH: usize = 64;

/// Why the capture table could not be read back.
enum Unread {
    /// The read itself failed: nothing is known about the table.
    Unavailable(Error),
    /// `nft list tables` succeeded and does not list the table.
    Missing,
}

pub(super) struct Backend {
    pub store: Store,
    pub ledger: Ledger,
    pub commands: Commands,
    pub handle: Handle,
    pub expected: Arc<Mutex<Expected>>,
    pub limit: usize,
    pub start: u32,
    pub monitored: bool,
}

/// Exact routes of the capture table for `gate`: the CIDR covers of its
/// managed IPv4 and IPv6 ranges through its device, each from the device's
/// Mesh address of that family, and none without one.
fn routes(table: u32, gate: &Gate) -> Vec<RouteMessage> {
    let Some(device) = &gate.device else {
        return Vec::new();
    };
    let four = device.source.into_iter().flat_map(|source| {
        gate.ranges.routes().map(move |net| {
            let mut builder = RouteMessageBuilder::<Ipv4Addr>::new();
            if net.prefix_len() != 0 {
                builder = builder.destination_prefix(net.network(), net.prefix_len());
            }
            builder
                .output_interface(device.index)
                .pref_source(source)
                .table_id(table)
                .priority(100)
                .protocol(PROTOCOL.into())
                .scope(RouteScope::Link)
                .kind(RouteType::Unicast)
                .build()
        })
    });
    let six = device.source6.into_iter().flat_map(|source| {
        gate.ranges.routes6().map(move |net| {
            let mut builder = RouteMessageBuilder::<Ipv6Addr>::new();
            if net.prefix_len() != 0 {
                builder = builder.destination_prefix(net.network(), net.prefix_len());
            }
            builder
                .output_interface(device.index)
                .pref_source(source)
                .table_id(table)
                .priority(100)
                .protocol(PROTOCOL.into())
                .scope(RouteScope::Universe)
                .kind(RouteType::Unicast)
                .build()
        })
    });
    four.chain(six).collect()
}

impl Backend {
    /// Open the recorded capture table, or create it. A retirement that
    /// deleted the table but did not read that back completes here
    /// (`ledger.removed`; nothing more to open); a table missing otherwise
    /// was removed by someone else.
    pub async fn initialize(&mut self) -> Result<()> {
        self.store.persist(&self.ledger).await?;
        if !self.present().await? {
            if self.ledger.receipt.is_some() {
                if self.ledger.retiring
                    && self.ledger.rules.is_empty()
                    && self.ledger.pending_gate.is_none()
                    && self.ledger.gate.device.is_none()
                    && self.table_routes().await?.is_empty()
                {
                    tracing::info!(
                        table = self.ledger.program.table(),
                        "Egress capture retirement completed from the kernel's state"
                    );
                    self.ledger.removed = true;
                    return self.store.persist(&self.ledger).await;
                }
                return Err(Error::Conflict);
            }
            let _result = objects::mutate(&mut self.commands, &self.ledger.program.create()).await;
        } else if self.ledger.retiring {
            // The deletion never took: the table is this owner's again.
            self.ledger.retiring = false;
            self.store.persist(&self.ledger).await?;
        }
        let mut observed = self.dump().await?;
        if self.ledger.receipt.is_some() && !self.ledger.program.inbound {
            observed = self.add_inbound(observed).await?;
        }
        if let Some(receipt) = &self.ledger.receipt {
            if observed.objects != receipt.objects || observed.handles != receipt.handles {
                return Err(Error::Conflict);
            }
            if !self.elements_match(&observed, &self.ledger.gate)
                && !self
                    .ledger
                    .pending_gate
                    .as_ref()
                    .is_some_and(|gate| self.elements_match(&observed, gate))
            {
                return Err(Error::Conflict);
            }
        } else {
            let intended = objects::decode(
                &serde_json::to_vec(&self.ledger.program.create()).map_err(|_| Error::Journal)?,
            )?;
            if observed.objects != intended.objects
                || observed.handles.len() != intended.objects.len()
            {
                #[cfg(test)]
                for (key, value) in &observed.objects {
                    if intended.objects.get(key) != Some(value) {
                        eprintln!(
                            "egress initial shape {key}: expected={:?} actual={value}",
                            intended.objects.get(key)
                        );
                        break;
                    }
                }
                return Err(Error::Conflict);
            }
            self.ledger.receipt = Some(observed);
            self.store.persist(&self.ledger).await?;
        }
        if self.monitored {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table = Some(self.ledger.table);
        }
        Ok(())
    }
    /// A table an earlier version created and recorded gains the
    /// inbound-reply chains in one nft transaction; the receipt is then the
    /// table read back with them, its earlier objects and handles unchanged.
    /// A table already read back with exactly them (a start after the
    /// transaction whose record was not written) is recorded as it is.
    /// Anything else is not this owner's table.
    async fn add_inbound(&mut self, mut observed: Receipt) -> Result<Receipt> {
        let receipt = self.ledger.receipt.clone().ok_or(Error::Journal)?;
        let mut program = self.ledger.program.clone();
        program.inbound = true;
        let intended =
            objects::decode(&serde_json::to_vec(&program.create()).map_err(|_| Error::Journal)?)?
                .objects;
        if observed.objects != intended {
            if observed.objects != receipt.objects || observed.handles != receipt.handles {
                return Err(Error::Conflict);
            }
            let _outcome = objects::mutate(&mut self.commands, &program.inbound_batch()).await;
            observed = self.dump().await?;
            if observed.objects == receipt.objects && observed.handles == receipt.handles {
                // Nothing applied: the table is unchanged and still this
                // owner's; the next opening adds the chains again.
                return Err(Error::Netlink);
            }
            if observed.objects != intended {
                return Err(Error::Conflict);
            }
        }
        if receipt
            .handles
            .iter()
            .any(|(key, handle)| observed.handles.get(key) != Some(handle))
        {
            return Err(Error::Conflict);
        }
        tracing::info!(
            table = program.table(),
            "Egress capture table gained the inbound-reply exception"
        );
        self.ledger.program = program;
        self.ledger.receipt = Some(observed.clone());
        self.store.persist(&self.ledger).await?;
        Ok(observed)
    }
    async fn present(&mut self) -> Result<bool> {
        use super::super::tool::nft::{TablesError, tables};
        let listed = tables(&mut self.commands)
            .await
            .map_err(|error| match error {
                TablesError::Unavailable => Error::Netlink,
                TablesError::Malformed => Error::Journal,
            })?;
        let name = self.ledger.program.table();
        Ok(listed
            .iter()
            .any(|(family, table)| family == "inet" && *table == name))
    }
    /// Read the capture table back. A read that fails (the tool's 2 s
    /// budget, an interrupted netlink dump, unreadable output) says nothing
    /// about the table; only a table that is listed as absent is missing.
    async fn read(&mut self) -> std::result::Result<Receipt, Unread> {
        self.commands
            .settle()
            .await
            .map_err(|_| Unread::Unavailable(Error::Netlink))?;
        use super::super::tool::nft::{TableUnread, table};
        let stdout = table(&mut self.commands, "inet", &self.ledger.program.table())
            .await
            .map_err(|unread| match unread {
                TableUnread::Missing => Unread::Missing,
                TableUnread::Unavailable => Unread::Unavailable(Error::Netlink),
            })?;
        objects::decode(&stdout).map_err(Unread::Unavailable)
    }
    async fn dump(&mut self) -> Result<Receipt> {
        self.read().await.map_err(|unread| match unread {
            Unread::Unavailable(error) => error,
            Unread::Missing => Error::Conflict,
        })
    }
    pub async fn verify(&mut self) -> Result<()> {
        self.verify_gate(self.ledger.gate.clone()).await
    }
    /// The periodic proof of the installed gate, classified.
    pub async fn check(&mut self) -> Verification {
        self.check_gate(&self.ledger.gate.clone()).await
    }
    /// The read-back after a failed change of the gate. The change's target
    /// is the recorded pending gate (written before any set or route of it);
    /// its start is the recorded gate. Each set and route of the capture
    /// table at one of the two is the change's own path; anything else
    /// differs.
    pub async fn reread(&mut self) -> Reread {
        let observed = match self.read().await {
            Ok(observed) => observed,
            Err(Unread::Unavailable(error)) => return Reread::Unavailable(error),
            Err(Unread::Missing) => return Reread::Changed("table_missing"),
        };
        let Some(receipt) = self.ledger.receipt.as_ref() else {
            return Reread::Changed("receipt_missing");
        };
        if observed.objects != receipt.objects || observed.handles != receipt.handles {
            return Reread::Changed("objects_changed");
        }
        let start = self.ledger.gate.clone();
        let target = self.ledger.pending_gate.clone();
        let sets_done = target
            .as_ref()
            .is_some_and(|target| self.elements_match(&observed, target));
        if !sets_done && !self.elements_match(&observed, &start) {
            return Reread::Changed("set_elements_changed");
        }
        let actual: BTreeSet<_> = match self.table_routes().await {
            Ok(routes) => routes.into_iter().map(encode_route).collect(),
            Err(error) => return Reread::Unavailable(error),
        };
        if !actual.is_subset(&self.owned_routes()) {
            return Reread::Changed("routes_changed");
        }
        let routes_done = target.as_ref().is_some_and(|target| {
            actual
                == routes(self.ledger.table, target)
                    .into_iter()
                    .map(encode_route)
                    .collect()
        });
        if sets_done && routes_done {
            Reread::Done
        } else {
            Reread::Start
        }
    }
    async fn verify_gate(&mut self, gate: Gate) -> Result<()> {
        self.check_gate(&gate).await.result()
    }
    async fn check_gate(&mut self, gate: &Gate) -> Verification {
        let observed = match self.read().await {
            Ok(observed) => observed,
            Err(Unread::Unavailable(error)) => return Verification::Unavailable(error),
            Err(Unread::Missing) => return Verification::Changed("table_missing"),
        };
        let Some(receipt) = self.ledger.receipt.as_ref() else {
            return Verification::Changed("receipt_missing");
        };
        if observed.objects != receipt.objects || observed.handles != receipt.handles {
            #[cfg(test)]
            for (key, value) in &observed.objects {
                if receipt.objects.get(key) != Some(value) {
                    eprintln!(
                        "egress shape {key} expected={:?} actual={value}",
                        receipt.objects.get(key)
                    );
                }
            }
            return Verification::Changed("objects_changed");
        }
        if !self.elements_match(&observed, gate) {
            return Verification::Changed("set_elements_changed");
        }
        Verification::Holds
    }
    /// Every set holds exactly the gate's elements; the readback count is
    /// part of the comparison.
    fn elements_match(&self, observed: &Receipt, gate: &Gate) -> bool {
        self.ledger
            .program
            .elements(
                &gate.ranges,
                &gate.mesh,
                gate.device.as_ref().map(|device| device.name.as_str()),
            )
            .into_iter()
            .all(|(name, expected)| {
                let key = format!("inet/{}/set/{name}", self.ledger.program.table());
                let actual = observed.elements.get(&key).cloned().unwrap_or_default();
                let same =
                    actual.len() == expected.len() && canonical(actual) == canonical(expected);
                #[cfg(test)]
                if !same {
                    eprintln!("egress set {name} differs");
                }
                same
            })
    }
    /// Route bytes the capture table may hold now: the current and pending gates'.
    fn owned_routes(&self) -> BTreeSet<Vec<u8>> {
        std::iter::once(&self.ledger.gate)
            .chain(self.ledger.pending_gate.iter())
            .flat_map(|gate| routes(self.ledger.table, gate))
            .map(encode_route)
            .collect()
    }
    /// The one transition of the capture sets and the capture table: the new
    /// gate is recorded first, then the sets are replaced in one batch and
    /// read back, then the table is brought to exactly the gate's routes. A
    /// managed destination not routed to the active device is rejected
    /// throughout, so either order of the two steps is closed.
    async fn gate(&mut self, gate: Gate) -> Result<()> {
        let observed = self.dump().await?;
        let receipt = self.ledger.receipt.as_ref().ok_or(Error::Journal)?;
        if observed.objects != receipt.objects || observed.handles != receipt.handles {
            return Err(Error::Conflict);
        }
        self.release_stale_routes(&gate).await?;
        self.ledger.pending_gate = Some(gate.clone());
        self.store.persist(&self.ledger).await?;
        // Sets read back at the gate already (a retry after a replacement
        // whose tool failed once the kernel had applied it) are not replaced
        // again: the change only reads and records them.
        if !self.elements_match(&observed, &gate) {
            let batch = self.ledger.program.replace(
                &gate.ranges,
                &gate.mesh,
                gate.device.as_ref().map(|device| device.name.as_str()),
            );
            let _outcome = objects::apply(&mut self.commands, &batch).await;
        }
        self.verify_gate(gate.clone()).await?;
        self.reconcile_routes(&gate).await?;
        self.ledger.gate = gate;
        self.ledger.pending_gate = None;
        self.store.persist(&self.ledger).await
    }
    /// An earlier change that did not finish may have left routes of its
    /// own gate in the table, and its pending record is about to be
    /// replaced: while they are still owned, remove those that neither the
    /// recorded gate nor `next` keeps (removal only closes).
    async fn release_stale_routes(&mut self, next: &Gate) -> Result<()> {
        let Some(pending) = &self.ledger.pending_gate else {
            return Ok(());
        };
        let table = self.ledger.table;
        let keep: BTreeSet<_> = routes(table, &self.ledger.gate)
            .into_iter()
            .chain(routes(table, next))
            .map(encode_route)
            .collect();
        if routes(table, pending)
            .into_iter()
            .map(encode_route)
            .all(|route| keep.contains(&route))
        {
            return Ok(());
        }
        let owned = self.owned_routes();
        let mut issued = 0;
        for route in self.table_routes().await? {
            let bytes = encode_route(route.clone());
            if !owned.contains(&bytes) {
                return Err(Error::Conflict);
            }
            if !keep.contains(&bytes) {
                self.expect_route(true, bytes)?;
                super::super::execute(self.handle.route().del(route).execute()).await?;
                issued += 1;
                if issued % ROUTE_BATCH == 0 {
                    self.observed().await?;
                }
            }
        }
        self.observed().await
    }
    /// Delete every owned route `gate` lacks and add every one it needs, then
    /// prove the table holds exactly its routes. Any other route in the table
    /// is foreign.
    async fn reconcile_routes(&mut self, gate: &Gate) -> Result<()> {
        let owned = self.owned_routes();
        let wanted = routes(self.ledger.table, gate);
        let wanted_bytes: BTreeSet<_> = wanted.iter().cloned().map(encode_route).collect();
        let actual: Vec<_> = self.table_routes().await?;
        for route in &actual {
            if !owned.contains(&encode_route(route.clone())) {
                return Err(Error::Conflict);
            }
        }
        let present: BTreeSet<_> = actual.iter().cloned().map(encode_route).collect();
        let mut issued = 0;
        for route in actual {
            let bytes = encode_route(route.clone());
            if !wanted_bytes.contains(&bytes) {
                self.expect_route(true, bytes)?;
                super::super::execute(self.handle.route().del(route).execute()).await?;
                issued += 1;
                if issued % ROUTE_BATCH == 0 {
                    self.observed().await?;
                }
            }
        }
        for route in wanted {
            let bytes = encode_route(route.clone());
            if !present.contains(&bytes) {
                self.expect_route(false, bytes)?;
                super::super::execute(self.handle.route().add(route).execute()).await?;
                issued += 1;
                if issued % ROUTE_BATCH == 0 {
                    self.observed().await?;
                }
            }
        }
        self.observed().await?;
        let after: BTreeSet<_> = self
            .table_routes()
            .await?
            .into_iter()
            .map(encode_route)
            .collect();
        if after != wanted_bytes {
            return Err(Error::Conflict);
        }
        Ok(())
    }
    /// Wait until the kernel monitor has seen every route event owed so far.
    /// Pacing mutations this way keeps its notification socket from
    /// overflowing, which would end the monitor.
    async fn observed(&self) -> Result<()> {
        if !self.monitored {
            return Ok(());
        }
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if self
                    .expected
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .egress_events
                    .is_empty()
                {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        // The monitor did not catch up: nothing was read, nothing proven.
        .map_err(|_| Error::Drain)?
    }
    fn expect_route(&self, deleted: bool, bytes: Vec<u8>) -> Result<()> {
        if self.monitored {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .expect_egress(deleted, bytes);
        }
        Ok(())
    }
    async fn table_routes(&self) -> Result<Vec<RouteMessage>> {
        Ok(super::super::get_routes(&self.handle, self.limit)
            .await?
            .into_iter()
            .filter(|route| route_table(route) == self.ledger.table)
            .collect())
    }
    /// Commit after the caller staged the negative union and resolved authority.
    pub async fn commit_staged(
        &mut self,
        plan: EgressCapturePlan,
        mesh: Ranges,
        mut device: Option<Device>,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let next = plan::parse(&plan)?;
        // A table without IPv6 capture rejects all IPv6: no route leads there.
        if !self.ledger.program.capture6
            && let Some(device) = &mut device
        {
            device.source6 = None;
        }
        check()?;
        if plan.unavailable.is_some() {
            return Err(Error::Target);
        }
        self.gate(Gate {
            ranges: next,
            mesh,
            device,
        })
        .await?;
        if let Err(error) = check() {
            self.pause().await?;
            return Err(error);
        }
        self.ledger.plan = plan;
        self.store.persist(&self.ledger).await
    }
    pub async fn stage(&mut self, plan: EgressCapturePlan, mesh: Ranges) -> Result<()> {
        let next = plan::parse(&plan)?;
        let mut union = plan::union(&self.ledger.gate.ranges, &next);
        if let Some(pending) = &self.ledger.pending_gate {
            union = plan::union(&union, &pending.ranges);
        }
        // Close the old/new union atomically, including OUTPUT paths that
        // would otherwise hit the host local/broadcast table before the RPDB.
        self.gate(Gate {
            ranges: union,
            mesh,
            device: None,
        })
        .await?;
        self.install_selector().await?;
        self.ledger.plan = plan;
        self.store.persist(&self.ledger).await
    }
    pub async fn pause(&mut self) -> Result<()> {
        let mut gate = self.ledger.gate.clone();
        if let Some(pending) = &self.ledger.pending_gate {
            gate.ranges = plan::union(&gate.ranges, &pending.ranges);
        }
        gate.device = None;
        self.gate(gate).await
    }
    pub async fn retire(&mut self) -> Result<()> {
        self.pause().await?;
        self.remove_rules().await?;
        self.verify().await?;
        // Recorded before the deletion: a table missing after this point
        // is the retirement's own (`initialize`), never a foreign removal.
        self.ledger.retiring = true;
        self.store.persist(&self.ledger).await?;
        let _outcome = objects::mutate(&mut self.commands, &json!({"nftables": [{"delete": {"table": {"family": "inet", "name": self.ledger.program.table()}}}]})).await;
        #[cfg(test)]
        if super::super::tool::command::faults::take(
            &super::super::tool::command::faults::FAILING_RETIRE_READS,
        ) {
            return Err(Error::Netlink);
        }
        if self.present().await? {
            return Err(Error::Conflict);
        }
        self.ledger.removed = true;
        self.store.persist(&self.ledger).await
    }
    /// The selector of each family the table captures: local-origin traffic
    /// without the protected mark looks up the capture table; a miss
    /// continues in the host RPDB.
    fn selectors(&self) -> Vec<RuleMessage> {
        let mut families = vec![AddressFamily::Inet];
        if self.ledger.program.capture6 {
            families.push(AddressFamily::Inet6);
        }
        families
            .into_iter()
            .map(|family| self.selector(family))
            .collect()
    }
    fn selector(&self, family: AddressFamily) -> RuleMessage {
        let mut rule = RuleMessage::default();
        rule.header.family = family;
        rule.header.action = RuleAction::ToTable;
        rule.attributes = vec![
            RuleAttribute::Priority(self.start),
            RuleAttribute::Table(self.ledger.table),
            RuleAttribute::Protocol(PROTOCOL.into()),
            RuleAttribute::Iifname("lo".into()),
            RuleAttribute::FwMask(self.ledger.program.mark),
        ];
        rule
    }
    async fn install_selector(&mut self) -> Result<()> {
        let selectors = self.selectors();
        if self.ledger.rules
            == selectors
                .iter()
                .cloned()
                .map(encode_rule)
                .collect::<Vec<_>>()
        {
            return Ok(());
        }
        self.remove_rules().await?;
        for selector in selectors {
            self.add_rule(selector).await?;
        }
        Ok(())
    }
    async fn add_rule(&mut self, rule: RuleMessage) -> Result<()> {
        let bytes = encode_rule(rule.clone());
        let actual = super::super::get_rules(&self.handle, self.limit).await?;
        if actual.iter().any(|other| {
            other.header.family == rule.header.family && priority(other) == priority(&rule)
        }) {
            return Err(Error::Conflict);
        }
        self.ledger.rules.push(bytes.clone());
        self.store.persist(&self.ledger).await?;
        if self.monitored {
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_rules
                .push(bytes);
        }
        self.objects().add_rule(rule).await
    }
    pub async fn remove_rules(&mut self) -> Result<()> {
        for bytes in self.ledger.rules.clone().into_iter().rev() {
            let objects = self.objects();
            let observed = objects.owned_rule(&bytes, None).await?;
            objects.remove_rule(&bytes, observed).await?;
            self.ledger.rules.retain(|rule| *rule != bytes);
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_rules
                .retain(|rule| *rule != bytes);
            self.store.persist(&self.ledger).await?;
        }
        Ok(())
    }
    fn objects(&self) -> super::super::rpdb::Objects<'_> {
        super::super::rpdb::Objects {
            handle: &self.handle,
            expected: &self.expected,
            limit: self.limit,
            monitored: self.monitored,
        }
    }
}

/// nft lists an interval as a single address, a prefix or a range; compare
/// the closed ranges they denote.
fn canonical(values: Vec<serde_json::Value>) -> Vec<String> {
    fn address(value: &serde_json::Value) -> Option<IpAddr> {
        value.as_str()?.parse().ok()
    }
    let mut values: Vec<_> = values
        .into_iter()
        .map(|value| {
            if let Some(prefix) = value.get("prefix")
                && let (Some(start), Some(len)) = (address(&prefix["addr"]), prefix["len"].as_u64())
                && let Ok(net) = ipnet::IpNet::new(start, len as u8)
            {
                return format!("{}-{}", net.network(), net.broadcast());
            }
            if let Some(range) = value.get("range").and_then(|r| r.as_array())
                && let [start, end] = range.as_slice()
                && let (Some(start), Some(end)) = (address(start), address(end))
            {
                return format!("{start}-{end}");
            }
            if let Some(ip) = address(&value) {
                return format!("{ip}-{ip}");
            }
            value.to_string()
        })
        .collect();
    values.sort();
    values
}
