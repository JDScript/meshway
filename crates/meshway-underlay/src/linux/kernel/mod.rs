mod anchor;
mod capture;
pub(crate) mod egress;
pub(crate) mod exit;
mod install;
mod journal;
mod monitor;
mod namespace;
pub(super) mod network;
mod plan;
mod proof;
mod repair;
mod rpdb;
pub(in crate::linux) mod rtnl;
mod term;
mod tool;
mod verification;

use futures_util::{TryStream, TryStreamExt};
use meshway_engine::Desired;
use meshway_private_storage::Directory;
use meshway_tun::Tun;
use rtnetlink::{
    IpVersion,
    packet_route::{
        AddressFamily,
        address::{AddressAttribute, AddressMessage},
        link::LinkMessage,
        route::RouteMessage,
        rule::{RuleAttribute, RuleMessage},
    },
};
use rtnl::Handle;
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
    /// Alive while this term's Kernel is: its request connections report
    /// their own end only meanwhile (first field: dropped before `handle`,
    /// whose drop ends the owner connection).
    _term: Arc<()>,
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
    /// A repair still owed: its reads failed before it could tell what is
    /// missing; its recovery says when it is due again.
    repair_owed: Option<meshway_model::recovery::Recovery>,
}

/// Repairs of removed capture objects allowed within [`REPAIR_WINDOW`].
const MAX_REPAIRS: usize = 3;
const REPAIR_WINDOW: Duration = Duration::from_secs(60);
/// How long the reads of one owed repair may fail, [`REPAIR_RETRY`] apart,
/// before ownership ends as unavailable (the egress owner's limit).
const REPAIR_READS_LIMIT: Duration = verification::UNVERIFIED_LIMIT;
const REPAIR_RETRY: Duration = Duration::from_secs(1);

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

    /// End this ownership term after it ended (`Shared::end`): withdraw
    /// every operation, join the capture and exit owners, close the owned
    /// TUN and settle the journal. Kernel objects stay as the journals record
    /// them; the Host's next term recovers them like a restart. The caller
    /// stops the term's tasks.
    pub async fn close_term(&mut self) {
        let _ = self.shared.suspend();
        self.dns_cancel.cancel();
        if let Some(mut owner) = self.egress.take() {
            self.clamp = None;
            let _ = owner.shutdown(false).await;
        }
        if let Some(mut owner) = self.exit.take() {
            owner.abandon().await;
        }
        self.probe = None;
        self.drop_owned_tun();
        let _ = self.store.settle().await;
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
        .map_err(|_| Error::Drain)?
    }

    /// End ownership after `operation` failed with `error`. An operation
    /// that failed in a term already ended (its monitor stopped, a
    /// connection dropped) failed because of that end and proves nothing
    /// more: the first cause stands, and a re-proof reads the kernel again.
    fn end_after(&self, operation: &'static str, error: &Error) {
        if self.shared.ended().is_none() {
            self.shared.end(ended(operation, error));
        }
    }
}

/// What a failed kernel operation's error proves. The one classification
/// every kernel owner uses (installation, repair, monitor, re-proof, the
/// egress capture owner's changes and the exit owner's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::linux) enum Kind {
    /// A read or change that failed, timed out or stopped, or the owner's
    /// own internal failure: nothing was read back, nothing is proven.
    Unknown,
    /// Read back and different from what the owner owns: a foreign object,
    /// a mark or journal that does not match.
    Proven,
    /// Refused before any kernel work (expired authority, an unsupported
    /// target, a full budget, invalid options): certain, no outcome in doubt.
    Refused,
}

pub(in crate::linux) fn kind(error: &Error) -> Kind {
    match error {
        Error::Conflict | Error::Protection | Error::Journal | Error::JournalVersion { .. } => {
            Kind::Proven
        }
        Error::Expired
        | Error::Target
        | Error::Capacity
        | Error::Options
        | Error::Origin
        | Error::Unsupported
        | Error::Resolver
        | Error::ControlUnavailable => Kind::Refused,
        Error::Netlink
        | Error::Io(_)
        | Error::Drain
        | Error::Worker
        | Error::Storage(_)
        | Error::Tun(_)
        | Error::Bind { .. } => Kind::Unknown,
    }
}

/// How an owner's failed kernel `operation` ends ownership: only a read-back
/// that differs is a conflict (`<operation>`); anything else proves nothing
/// (`<operation>_unavailable`, `drain_unavailable` for a missed drain) and
/// is re-proved. A refusal that ends ownership left a change of its own in
/// doubt (a pending journal step): unavailable as well.
pub(in crate::linux) fn ended(operation: &'static str, error: &Error) -> crate::OwnershipEnd {
    if kind(error) == Kind::Proven {
        return crate::OwnershipEnd::Conflict { check: operation };
    }
    let check = match (error, operation) {
        (Error::Drain, _) => "drain_unavailable",
        (_, "repair") => "repair_unavailable",
        (_, "monitor") => "monitor_unavailable",
        (_, "retire") => "retire_unavailable",
        (_, "pause") => "pause_unavailable",
        (_, "stage") => "stage_unavailable",
        (_, "switch") => "switch_unavailable",
        _ => "install_unavailable",
    };
    crate::OwnershipEnd::Unavailable { check }
}

/// The answers of one request that is not a dump (dumps: [`rtnl`]).
async fn collect<S>(mut stream: S, limit: usize) -> Result<Vec<S::Ok>>
where
    S: TryStream<Error = rtnetlink::Error> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut result = Vec::new();
        while let Some(message) = stream
            .try_next()
            .await
            .map_err(|error| netlink_failure("dump", error))?
        {
            if result.len() == limit {
                return Err(Error::Capacity);
            }
            result.push(message);
        }
        Ok(result)
    })
    .await
    .map_err(|_| netlink_timeout("dump"))?
}

async fn execute(
    operation: impl std::future::Future<Output = std::result::Result<(), rtnetlink::Error>>,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .map_err(|_| netlink_timeout("request"))?
        .map_err(|error| netlink_failure("request", error))
}

/// The kernel's own reason for a failed netlink operation, at debug: callers
/// only see `Error::Netlink`.
fn netlink_failure(operation: &'static str, error: rtnetlink::Error) -> Error {
    match &error {
        rtnetlink::Error::NetlinkError(message) => tracing::debug!(
            operation,
            errno = -message.raw_code(),
            "Kernel netlink operation failed"
        ),
        error => tracing::debug!(operation, %error, "Kernel netlink operation failed"),
    }
    Error::Netlink
}

fn netlink_timeout(operation: &'static str) -> Error {
    tracing::debug!(operation, "Kernel netlink operation timed out");
    Error::Netlink
}

async fn get_rules(handle: &Handle, limit: usize) -> Result<Vec<RuleMessage>> {
    #[cfg(test)]
    if tool::command::faults::take(&tool::command::faults::FAILING_RULE_READS) {
        return Err(Error::Netlink);
    }
    let mut rules = rtnl::rules(handle, IpVersion::V4, limit).await?;
    rules.extend(rtnl::rules(handle, IpVersion::V6, limit.saturating_sub(rules.len())).await?);
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
        routes.extend(rtnl::routes(handle, family, limit.saturating_sub(routes.len())).await?);
    }
    Ok(routes)
}

async fn get_addresses(handle: &Handle, limit: usize) -> Result<Vec<AddressMessage>> {
    rtnl::addresses(handle, limit).await
}

async fn get_links(handle: &Handle, limit: usize) -> Result<Vec<LinkMessage>> {
    rtnl::links(handle, limit).await
}

#[cfg(test)]
mod tests;
