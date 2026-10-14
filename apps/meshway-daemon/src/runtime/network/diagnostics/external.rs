//! `meshway ping` toward a target outside the Mesh: resolve a name through
//! this Node's DNS path, report the local decision, then echo. An exit
//! decision sends ordinary local packets that SecureNode decides, carries
//! and answers like any TUN packet; direct and unmanaged ones echo from this
//! host's own network. Timing belongs to the dataplane echo owner.
use crate::runtime::PingRequest;
use crate::runtime::ping::Progress;
use meshway_dataplane::diagnostics::{decision, echo};
use meshway_dataplane::secure::SecureNode;
use meshway_dataplane::{Packet, PacketFacts, PendingAction};
use meshway_engine::Desired;
use meshway_protocol::ipc::{
    PingAction, PingEcho, PingEchoOutcome, PingError, PingEvent, PingIcmpError, PingResolution,
    PingResolver,
};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};
use tokio::sync::watch;

/// A name lookup through the system resolver, as any application's.
const RESOLUTION_TIMEOUT: Duration = Duration::from_secs(10);

struct Resolving {
    request: PingRequest,
    progress: watch::Sender<Progress>,
    name: String,
    resolver: PingResolver,
}

/// An echo through the Mesh on the exit path.
struct Mesh {
    request: PingRequest,
    _progress: watch::Sender<Progress>,
    echo: echo::Echo,
    counts: Counts,
}

#[derive(Default)]
pub(super) struct Counts {
    pub sent: u32,
    pub success: u32,
    pub timeout: u32,
    pub failed: u32,
    pub results: u16,
}

impl Counts {
    /// Count one result and return its event.
    pub fn record(&mut self, result: &echo::Result, path: Vec<meshway_model::NodeId>) -> PingEvent {
        self.results = self.results.saturating_add(1);
        let (outcome, error) = match result.outcome {
            echo::Outcome::Reply => {
                self.sent += 1;
                self.success += 1;
                (PingEchoOutcome::Reply, None)
            }
            echo::Outcome::Unreachable { kind, code } => {
                self.sent += 1;
                (
                    PingEchoOutcome::Unreachable,
                    Some(PingIcmpError { kind, code }),
                )
            }
            echo::Outcome::Timeout => {
                self.sent += 1;
                self.timeout += 1;
                (PingEchoOutcome::Timeout, None)
            }
            echo::Outcome::Failed => {
                self.failed += 1;
                (PingEchoOutcome::Failed, None)
            }
        };
        PingEvent::Echo {
            echo: PingEcho {
                sequence: result.sequence,
                outcome,
                from: result.from.map(|from| from.to_string()),
                error,
                duration_us: result
                    .rtt
                    .and_then(|rtt| u64::try_from(rtt.as_micros()).ok())
                    .and_then(|us| meshway_model::Sequence::new(us).ok()),
                setup: result.setup,
                path,
            },
        }
    }

    pub fn completed(&self) -> PingEvent {
        PingEvent::Completed {
            sent: self.sent,
            success: self.success,
            timeout: self.timeout,
            failed: self.failed,
        }
    }
}

#[derive(Default)]
pub(in crate::runtime::network) struct Externals {
    next: u64,
    resolving: BTreeMap<u64, Resolving>,
    lookups: tokio::task::JoinSet<(u64, Result<Vec<IpAddr>, ()>)>,
    mesh: BTreeMap<u64, Mesh>,
    direct: tokio::task::JoinSet<()>,
}

fn fail(request: &PingRequest, code: PingError) {
    let _ = request.events.try_send(PingEvent::Failed { code });
    request.cancellation.cancel();
}

fn random_identifier() -> Option<u16> {
    let mut bytes = [0u8; 2];
    getrandom::fill(&mut bytes).ok()?;
    Some(u16::from_be_bytes(bytes))
}

impl Externals {
    pub fn len(&self) -> usize {
        self.resolving.len() + self.mesh.len() + self.direct.len()
    }

    /// Resolve `name` through this Node's own DNS path first.
    pub fn resolve(
        &mut self,
        desired: &Desired,
        request: PingRequest,
        progress: watch::Sender<Progress>,
        name: String,
    ) {
        let resolver = match meshway_dns::name_route(desired, &name) {
            meshway_dns::NameRoute::MagicDns => PingResolver::MagicDns,
            meshway_dns::NameRoute::Exit(node) => PingResolver::Exit { node },
            meshway_dns::NameRoute::Resolvers(keys) => PingResolver::Resolvers { keys },
            meshway_dns::NameRoute::System => PingResolver::System,
            meshway_dns::NameRoute::Unavailable => PingResolver::Unavailable,
        };
        // Resolution waits for the DNS path, not for a circuit setup.
        progress.send_if_modified(|old| old.replace(Progress::Waiting));
        let key = self.next;
        self.next += 1;
        let lookup = name.clone();
        let cancellation = request.cancellation.clone();
        self.lookups.spawn(async move {
            let found = tokio::select! {
                _ = cancellation.cancelled() => Err(()),
                found = tokio::time::timeout(RESOLUTION_TIMEOUT, tokio::net::lookup_host((lookup.as_str(), 0))) => {
                    match found {
                        Ok(Ok(addresses)) => {
                            let mut unique = Vec::new();
                            for address in addresses.map(|address| address.ip()) {
                                if !unique.contains(&address) { unique.push(address); }
                            }
                            Ok(unique)
                        }
                        _ => Err(()),
                    }
                }
            };
            (key, found)
        });
        self.resolving.insert(
            key,
            Resolving {
                request,
                progress,
                name,
                resolver,
            },
        );
    }

    /// Decide `destination` and start its echo.
    pub fn start(
        &mut self,
        secure: &mut SecureNode,
        request: PingRequest,
        progress: watch::Sender<Progress>,
        destination: IpAddr,
        resolution: Option<PingResolution>,
    ) {
        let now_ms = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default();
        let decided = decision::decide(secure, destination, resolution, now_ms);
        let action = decided.action.clone();
        tracing::info!(request = %request.id, %destination, action = ?action,
            path = ?decided.path, "Local ping decided outside the Mesh");
        if request
            .events
            .try_send(PingEvent::Decision { decision: decided })
            .is_err()
        {
            request.cancellation.cancel();
            return;
        }
        progress.send_if_modified(|old| old.replace(Progress::Waiting));
        let Some(identifier) = random_identifier() else {
            return fail(&request, PingError::Failed);
        };
        match (action, destination) {
            (Some(PingAction::Exit { .. }), IpAddr::V4(target)) => {
                let key = self.next;
                self.next += 1;
                self.mesh.insert(
                    key,
                    Mesh {
                        echo: echo::Echo::new(target, identifier, Instant::now()),
                        request,
                        _progress: progress,
                        counts: Counts::default(),
                    },
                );
            }
            (Some(PingAction::Direct | PingAction::Unmanaged), IpAddr::V4(target)) => {
                self.direct
                    .spawn(super::direct::run(target, identifier, request, progress));
            }
            (Some(PingAction::Denied { .. }), _) => fail(&request, PingError::PermissionDenied),
            _ => fail(&request, PingError::Unsupported),
        }
    }

    /// Join finished lookups and time out in-flight echoes.
    pub fn drive(&mut self, secure: Option<&mut SecureNode>) {
        let mut secure = secure;
        while let Some(joined) = self.lookups.try_join_next() {
            let Ok((key, found)) = joined else { continue };
            let Some(resolving) = self.resolving.remove(&key) else {
                continue;
            };
            let Resolving {
                request,
                progress,
                name,
                resolver,
            } = resolving;
            if request.cancellation.is_cancelled() {
                continue;
            }
            let addresses = found.unwrap_or_default();
            let resolution = PingResolution {
                name,
                resolver,
                addresses: addresses.iter().map(ToString::to_string).collect(),
                association: None,
            };
            // Exit execution is IPv4: the first IPv4 answer, else the first.
            let Some(destination) = addresses
                .iter()
                .find(|address| address.is_ipv4())
                .or(addresses.first())
                .copied()
            else {
                let _ = request.events.try_send(PingEvent::Decision {
                    decision: decision::unresolved(resolution),
                });
                fail(&request, PingError::TargetNotFound);
                continue;
            };
            match secure.as_deref_mut() {
                Some(secure) => {
                    self.start(secure, request, progress, destination, Some(resolution))
                }
                None => fail(&request, PingError::NotRunning),
            }
        }
        self.resolving
            .retain(|_, resolving| !resolving.request.cancellation.is_cancelled());
        let now = Instant::now();
        for ping in self.mesh.values_mut() {
            if let Some(result) = ping.echo.expire(now) {
                let event = ping.counts.record(&result, Vec::new());
                report(&ping.request, &mut ping.counts, event);
            }
            if ping.request.events.is_closed() {
                ping.request.cancellation.cancel();
            }
        }
        self.mesh
            .retain(|_, ping| !ping.request.cancellation.is_cancelled());
        while self.direct.try_join_next().is_some() {}
    }

    /// Requests due now, as local packets from this Node's Mesh `source`.
    pub fn packets(&mut self, source: Option<Ipv4Addr>) -> Vec<Packet> {
        let now = Instant::now();
        let mut packets = Vec::new();
        for ping in self.mesh.values_mut() {
            let count = ping.request.count;
            if ping.request.cancellation.is_cancelled()
                || ping.counts.results.saturating_add(1) > count
                || !ping.echo.due(now)
            {
                continue;
            }
            let packet = source.and_then(|source| ping.echo.packet(source, now).ok());
            match packet {
                Some(packet) => packets.push(packet),
                None => {
                    // Without its Mesh address this Node sends nothing.
                    if let Some(result) = ping.echo.refused(now) {
                        let event = ping.counts.record(&result, Vec::new());
                        report(&ping.request, &mut ping.counts, event);
                    }
                }
            }
        }
        packets
    }

    /// The in-flight request `facts` describe, if any.
    pub fn request(&self, facts: &PacketFacts) -> Option<u64> {
        self.mesh
            .iter()
            .find(|(_, ping)| ping.echo.is_request(facts))
            .map(|(key, _)| *key)
    }

    /// SecureNode's admission of a request: committed now, queued behind a
    /// circuit setup, or refused.
    pub fn admitted(&mut self, key: u64, admitted: Option<bool>) {
        let Some(ping) = self.mesh.get_mut(&key) else {
            return;
        };
        let now = Instant::now();
        match admitted {
            Some(setup) => ping.echo.admitted(now, setup),
            None => {
                if let Some(result) = ping.echo.refused(now) {
                    let event = ping.counts.record(&result, Vec::new());
                    report(&ping.request, &mut ping.counts, event);
                }
            }
        }
    }

    /// Consume a delivered packet answering an in-flight request.
    pub fn answer(&mut self, packet: &PendingAction) -> bool {
        let facts = packet.facts();
        let now = Instant::now();
        for ping in self.mesh.values_mut() {
            if let Some(result) = ping.echo.answer(&facts, now) {
                let path = packet.route().original_path.clone();
                let event = ping.counts.record(&result, path);
                report(&ping.request, &mut ping.counts, event);
                return true;
            }
        }
        false
    }

    pub fn wake_at(&self) -> Option<Instant> {
        let lookups = (!self.resolving.is_empty() || !self.direct.is_empty())
            .then(|| Instant::now() + Duration::from_millis(100));
        self.mesh
            .values()
            .map(|ping| ping.echo.wake_at())
            .chain(lookups)
            .min()
    }

    pub async fn stop(&mut self) {
        for resolving in self.resolving.values() {
            fail(&resolving.request, PingError::Stopped);
        }
        for ping in self.mesh.values() {
            fail(&ping.request, PingError::Stopped);
        }
        self.resolving.clear();
        self.mesh.clear();
        self.lookups.abort_all();
        while self.lookups.join_next().await.is_some() {}
        // Direct echoes end with their request's cancellation.
        self.direct.abort_all();
        while self.direct.join_next().await.is_some() {}
    }
}

/// Deliver one result and complete the ping after its last one.
pub(super) fn report(request: &PingRequest, counts: &mut Counts, event: PingEvent) {
    if request.events.try_send(event).is_err() {
        request.cancellation.cancel();
        return;
    }
    if counts.results >= request.count {
        let _ = request.events.try_send(counts.completed());
        request.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests {
    /// A direct echo task that would run on (it waits on nothing its request
    /// cancels) is aborted and joined by the withdrawal.
    #[tokio::test]
    async fn withdrawal_ends_and_joins_direct_echo_tasks() {
        struct Dropped(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let mut diagnostics = super::super::Diagnostics::default();
        let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        diagnostics.externals.direct.spawn(async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        assert_eq!(diagnostics.externals.len(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(2), diagnostics.withdraw())
            .await
            .unwrap();
        assert_eq!(diagnostics.externals.len(), 0);
        assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
    }
}
