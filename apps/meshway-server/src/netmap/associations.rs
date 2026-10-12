//! Trusted DNS associations: the only per-name compilation the Server keeps.
//!
//! The task is standing and Server-issued: an exit's signed netmap names
//! every source it resolves for (`ServeExitResolver`), bound to both Nodes'
//! netmaps, its revision and its lease. When that exit answers a query whose
//! name a name rule of the source decides, it completes the task once
//! (`POST /agent-api/v1/dns-associations`, single-use `reportId`). The Server
//! checks the binding, the rule, the addresses and the TTL against both
//! current netmaps, keeps the association in memory (bounded per source) and
//! wakes both Nodes' syncs; each receives a signed overlay for its own netmap.
//! A netmap change of either Node revalidates every association against the
//! new content before it is delivered again; a Server restart forgets them
//! (fail closed: the Nodes clear theirs on their next answer).
use crate::{error::Error, secrets::Sealer, store::netmap as store};
use meshway_configuration::netmap::{
    CompiledNetmap, DnsAssociation, DnsAssociationOverlay, DnsPlan, DnsService,
    MAX_ASSOCIATION_ADDRESSES, MAX_ASSOCIATION_CNAMES, MAX_ASSOCIATION_LIFETIME_MS,
    MAX_OVERLAY_ASSOCIATIONS, MAX_SOURCE_ASSOCIATIONS, MIN_ASSOCIATION_LIFETIME_MS,
};
use meshway_model::{NetmapRef, NetworkId, NodeId, RequestId};
use meshway_policy::compile::{
    Artifacts,
    netmap::{Associations, LocalCompiler, associable},
    rules::query_name,
};
use meshway_protocol::{DnsAssociationAccepted, DnsAssociationReport};
use rusqlite::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex},
};

/// Sources remembered at most; the one whose latest entry ends first goes.
const MAX_SOURCES: usize = 65_536;
/// Completed report IDs remembered (single use) at most.
const MAX_REPORTS: usize = 65_536;
/// A completion must arrive within this long after resolution.
const MAX_REPORT_AGE_MS: u64 = 30_000;
/// Clock skew tolerated for a resolution time ahead of the Server.
const MAX_REPORT_SKEW_MS: u64 = 5_000;

type Key = (NetworkId, NodeId);

/// Associations of one source, all through one exit and one selection.
#[derive(Clone)]
struct Source {
    exit: NodeId,
    service: DnsService,
    /// Netmaps of the source and the exit they were last checked against.
    checked: (NetmapRef, NetmapRef),
    entries:
        BTreeMap<(String, meshway_configuration::netmap::AssociationRecordType), DnsAssociation>,
}

#[derive(Default)]
struct State {
    sources: HashMap<Key, Source>,
    reports: VecDeque<(RequestId, u64)>,
    seen: BTreeSet<RequestId>,
}

#[derive(Default)]
pub(crate) struct Store {
    state: Mutex<State>,
}

/// Both current netmaps of one (source, exit) pair and the service binding them.
struct Pair {
    source: CompiledNetmap,
    exit: CompiledNetmap,
    service: DnsService,
}

fn current(tx: &Transaction<'_>, network: NetworkId, node: NodeId) -> Result<NetmapRef, Error> {
    Ok(store::current(tx, network, node)?
        .ok_or(Error::StateScopeChanged)?
        .current)
}

/// The exit's current netmap serves `source` with the exact service the
/// source's current netmap resolves through.
fn pair(
    tx: &Transaction<'_>,
    sealer: &Sealer,
    network: NetworkId,
    source: NodeId,
    exit: NodeId,
) -> Result<Option<Pair>, Error> {
    let (Some(source_ref), Some(exit_ref)) = (
        store::current(tx, network, source)?,
        store::current(tx, network, exit)?,
    ) else {
        return Ok(None);
    };
    let source_netmap = store::load(tx, sealer, network, source, source_ref.current)?;
    let exit_netmap = store::load(tx, sealer, network, exit, exit_ref.current)?;
    let DnsPlan::ViaExit { service } = &source_netmap.netmap.dns.egress else {
        return Ok(None);
    };
    let served = matches!(&exit_netmap.netmap.dns.egress,
        DnsPlan::ServeExitResolver { services } if services.contains(service));
    if service.exit != exit || service.source != source || !served {
        return Ok(None);
    }
    let service = (**service).clone();
    Ok(Some(Pair {
        source: source_netmap,
        exit: exit_netmap,
        service,
    }))
}

/// The source's compiler over every artifact its netmap references.
fn compiler(tx: &Transaction<'_>, netmap: &CompiledNetmap) -> Result<LocalCompiler, Error> {
    let mut artifacts = Artifacts::new();
    for reference in netmap.netmap.projection.rule_sets.values() {
        let set = crate::resources::artifacts::load(tx, &reference.digest)?;
        artifacts.insert(reference.digest, set);
    }
    Ok(LocalCompiler::from_shared(
        netmap.netmap.clone(),
        &artifacts,
    ))
}

fn overlay_of(netmap: NetmapRef, entries: Vec<DnsAssociation>) -> DnsAssociationOverlay {
    DnsAssociationOverlay {
        netmap,
        associations: entries,
    }
}

impl Source {
    /// Live entries in overlay order.
    fn live(&self, now_ms: u64) -> Vec<DnsAssociation> {
        self.entries
            .values()
            .filter(|entry| entry.expires_at_ms > now_ms)
            .cloned()
            .collect()
    }

    /// Drop what `compiler` no longer decides by name and what no longer fits
    /// the source's capture.
    fn retain_valid(&mut self, compiler: &LocalCompiler, now_ms: u64) {
        let source = self.service.source;
        self.entries.retain(|(name, _), entry| {
            entry.expires_at_ms > now_ms
                && compiler
                    .egress_rule_for_name(source, name)
                    .is_ok_and(|hit| hit.is_some())
        });
    }
}

/// Whether the source's executed capture stays within its bounds with `entries`.
fn fits(compiler: &LocalCompiler, source: &CompiledNetmap, entries: &[DnsAssociation]) -> bool {
    let Some(plan) = &source.netmap.egress else {
        return false;
    };
    let associations =
        Associations::new(Some(&overlay_of(source.reference(), entries.to_vec())), 0);
    compiler
        .associated_egress(source.netmap.node_id, plan, &associations)
        .is_ok_and(|plan| plan.capture.unavailable.is_none())
}

impl Store {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Complete the standing task of `exit` for `report.source` once.
    /// Returns the accepted association and the Nodes whose overlay changed.
    pub(crate) fn complete(
        &self,
        tx: &Transaction<'_>,
        sealer: &Sealer,
        network: NetworkId,
        exit: NodeId,
        report: &DnsAssociationReport,
        now_ms: u64,
    ) -> Result<(DnsAssociationAccepted, BTreeSet<NodeId>), Error> {
        if current(tx, network, exit)? != report.netmap {
            return Err(Error::StateScopeChanged);
        }
        let Some(pair) = pair(tx, sealer, network, report.source, exit)? else {
            return Err(Error::PermissionDenied);
        };
        if pair.service.generation != report.generation {
            return Err(Error::StateScopeChanged);
        }
        let name = query_name(&report.name);
        let addresses = &report.addresses;
        if name != report.name
            || name.is_empty()
            || report.cnames.len() > MAX_ASSOCIATION_CNAMES
            || report
                .cnames
                .iter()
                .any(|cname| query_name(cname) != *cname || cname.is_empty())
            || addresses.is_empty()
            || addresses.len() > MAX_ASSOCIATION_ADDRESSES
            || !addresses.windows(2).all(|pair| pair[0].0 < pair[1].0)
            || !addresses
                .iter()
                .all(|address| report.record_type.admits(address.0) && associable(address.0))
            || report.resolved_at_ms > now_ms + MAX_REPORT_SKEW_MS
            || report.resolved_at_ms + MAX_REPORT_AGE_MS < now_ms
        {
            return Err(Error::InvalidRequest);
        }
        let compiler = compiler(tx, &pair.source)?;
        if compiler
            .egress_rule_for_name(report.source, &name)
            .map_err(|_| Error::DependencyUnavailable)?
            .is_none()
        {
            return Err(Error::PermissionDenied);
        }
        let lifetime = (u64::from(report.ttl_seconds) * 1000)
            .clamp(MIN_ASSOCIATION_LIFETIME_MS, MAX_ASSOCIATION_LIFETIME_MS);
        let expires_at_ms = report.resolved_at_ms.saturating_add(lifetime);
        if expires_at_ms <= now_ms {
            return Err(Error::InvalidRequest);
        }
        let association = DnsAssociation {
            source: report.source,
            generation: report.generation,
            name: name.clone(),
            record_type: report.record_type,
            cnames: report.cnames.clone(),
            addresses: addresses.clone(),
            expires_at_ms,
        };
        let mut state = self.state();
        if state.seen.contains(&report.report_id) {
            return Err(Error::RequestConflict);
        }
        let key = (network, report.source);
        let mut next = state
            .sources
            .get(&key)
            .filter(|known| known.exit == exit && known.service == pair.service)
            .cloned()
            .unwrap_or(Source {
                exit,
                service: pair.service.clone(),
                checked: (pair.source.reference(), pair.exit.reference()),
                entries: BTreeMap::new(),
            });
        next.checked = (pair.source.reference(), pair.exit.reference());
        next.retain_valid(&compiler, now_ms);
        next.entries.insert((name, report.record_type), association);
        while next.entries.len() > MAX_SOURCE_ASSOCIATIONS {
            let earliest = next
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at_ms)
                .map(|(key, _)| key.clone());
            match earliest {
                Some(key) => next.entries.remove(&key),
                None => break,
            };
        }
        if !fits(&compiler, &pair.source, &next.live(now_ms)) {
            return Err(Error::StateLimit);
        }
        if state.sources.len() >= MAX_SOURCES && !state.sources.contains_key(&key) {
            let oldest = state
                .sources
                .iter()
                .min_by_key(|(_, source)| source.entries.values().map(|e| e.expires_at_ms).max())
                .map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                state.sources.remove(&oldest);
            }
        }
        state.sources.insert(key, next);
        while state.reports.len() >= MAX_REPORTS
            || state
                .reports
                .front()
                .is_some_and(|(_, until)| *until <= now_ms)
        {
            let Some((id, _)) = state.reports.pop_front() else {
                break;
            };
            state.seen.remove(&id);
        }
        state.reports.push_back((
            report.report_id,
            now_ms + MAX_REPORT_AGE_MS + MAX_REPORT_SKEW_MS,
        ));
        state.seen.insert(report.report_id);
        Ok((
            DnsAssociationAccepted { expires_at_ms },
            BTreeSet::from([report.source, exit]),
        ))
    }

    /// The overlay of `node` for its current netmap `netmap`: its own
    /// associations as a source and those of every source it resolves for,
    /// each pair revalidated when either netmap changed since its check.
    /// `None` when there is none.
    pub(crate) fn overlay(
        &self,
        tx: &Transaction<'_>,
        sealer: &Sealer,
        network: NetworkId,
        node: NodeId,
        netmap: NetmapRef,
        now_ms: u64,
    ) -> Result<Option<DnsAssociationOverlay>, Error> {
        let keys: Vec<(Key, NodeId)> = self
            .state()
            .sources
            .iter()
            .filter(|((owner, source), entry)| {
                *owner == network && (*source == node || entry.exit == node)
            })
            .map(|(key, entry)| (*key, entry.exit))
            .collect();
        let mut associations = Vec::new();
        for (key, exit) in keys {
            let refs = (
                current(tx, network, key.1).ok(),
                current(tx, network, exit).ok(),
            );
            let stale = self
                .state()
                .sources
                .get(&key)
                .is_none_or(|entry| (Some(entry.checked.0), Some(entry.checked.1)) != refs);
            if stale {
                let pair = pair(tx, sealer, network, key.1, exit)?;
                let checked = match &pair {
                    Some(pair) => {
                        let compiler = compiler(tx, &pair.source)?;
                        let mut state = self.state();
                        match state.sources.get_mut(&key) {
                            Some(entry) if entry.exit == exit && entry.service == pair.service => {
                                entry.checked = (pair.source.reference(), pair.exit.reference());
                                entry.retain_valid(&compiler, now_ms);
                                (!entry.entries.is_empty()
                                    && fits(&compiler, &pair.source, &entry.live(now_ms)))
                                .then_some(())
                            }
                            _ => None,
                        }
                    }
                    None => None,
                };
                if checked.is_none() {
                    self.state().sources.remove(&key);
                    continue;
                }
            }
            if let Some(entry) = self.state().sources.get(&key) {
                associations.extend(entry.live(now_ms));
            }
        }
        if associations.is_empty() {
            return Ok(None);
        }
        associations.sort_by(|a, b| {
            (a.source, &a.name, a.record_type).cmp(&(b.source, &b.name, b.record_type))
        });
        associations.truncate(MAX_OVERLAY_ASSOCIATIONS);
        Ok(Some(overlay_of(netmap, associations)))
    }
}

/// Shared by every sync answer and report of this process.
pub(crate) type Shared = Arc<Store>;
