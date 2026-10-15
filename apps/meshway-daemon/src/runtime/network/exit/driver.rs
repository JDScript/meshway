use super::{Health, Work, scope::PacketGuard};
use meshway_dataplane::exit::{self, ExitProjection, FlowIncarnation};
use meshway_protocol::ipc::Problem;
use meshway_underlay::{CtConfirmation, ExitIncarnation, ExitRole, ExitScope};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

struct Resource {
    revoked: CancellationToken,
    confirmed: bool,
    /// A confirmation of this flow's conntrack entry is outstanding.
    confirming: bool,
}

/// One flow's outstanding conntrack confirmation and the scope of the packet
/// that started it; it never holds the work queue.
type Confirmation = (FlowIncarnation, exit::ProjectionScope, bool);

enum Event {
    Work(Work),
    Confirmed(Result<Confirmation, tokio::task::JoinError>),
    /// The kernel answered the running operation.
    Done(Done),
    /// The collecting round waited long enough.
    RoundDue,
    /// The FIFO is closed; held work and the operation still finish.
    Closed,
    /// The role's owner stopped by itself.
    Ended,
}

/// At most this many installations share one kernel round.
const ROUND: usize = 64;
/// A round collects the installations that follow its first one closely
/// (the new flows of one burst, e.g. one traceroute's probes): it closes once
/// none arrived for `ROUND_GAP`, and at most `ROUND_WAIT` after it opened.
/// That bounded wait is what a lone new flow pays for sharing.
const ROUND_GAP: Duration = Duration::from_millis(3);
const ROUND_WAIT: Duration = Duration::from_millis(10);

struct Installing {
    flow: FlowIncarnation,
    tuple: exit::ExitTuple,
    scope: exit::ProjectionScope,
    ack: exit::ProjectionAck,
}

struct Outcome {
    item: Installing,
    revoked: CancellationToken,
    installed: bool,
}

/// What the kernel answered for one operation.
enum Done {
    Installed(Vec<Outcome>),
    Retired {
        ack: exit::RetirementAck,
        removed: bool,
    },
    Renewed {
        flow: FlowIncarnation,
        ack: exit::ProjectionAck,
        revoked: CancellationToken,
        renewed: bool,
    },
}

struct Collecting {
    items: Vec<Installing>,
    closes: tokio::time::Instant,
    idle: tokio::time::Instant,
}

/// The one kernel operation in progress, in FIFO order: an install round
/// (collecting, then running), a retirement or a renewal. Only work that
/// must follow it waits for it: packets of the flows it names, and every
/// later projection or barrier (a collecting round still takes new flows).
/// Packets of other, installed flows are written meanwhile, so a kernel
/// operation never holds a running flow.
struct Operation {
    kind: &'static str,
    started: tokio::time::Instant,
    flows: BTreeSet<FlowIncarnation>,
    collecting: Option<Collecting>,
    running: Option<Pin<Box<dyn Future<Output = Done> + Send>>>,
}

impl Operation {
    fn round(first: Installing) -> Self {
        let now = tokio::time::Instant::now();
        Self {
            kind: "install",
            started: now,
            flows: BTreeSet::from([first.flow]),
            collecting: Some(Collecting {
                items: vec![first],
                closes: now + ROUND_WAIT,
                idle: now + ROUND_GAP,
            }),
            running: None,
        }
    }
    fn running(
        kind: &'static str,
        flow: FlowIncarnation,
        done: impl Future<Output = Done> + Send + 'static,
    ) -> Self {
        Self {
            kind,
            started: tokio::time::Instant::now(),
            flows: BTreeSet::from([flow]),
            collecting: None,
            running: Some(Box::pin(done)),
        }
    }
    fn due(&self) -> Option<tokio::time::Instant> {
        self.collecting
            .as_ref()
            .map(|round| round.idle.min(round.closes))
    }
    fn joinable(&self) -> bool {
        self.collecting.is_some() && self.flows.len() < ROUND
    }
    fn join(&mut self, item: Installing) {
        if let Some(round) = &mut self.collecting {
            self.flows.insert(item.flow);
            round.items.push(item);
            round.idle = tokio::time::Instant::now() + ROUND_GAP;
        }
    }
    /// Close a collecting round and hand it to the kernel.
    fn submit(
        &mut self,
        role: &ExitRole,
        resources: &BTreeMap<FlowIncarnation, Resource>,
        stop: &CancellationToken,
    ) {
        let Some(round) = self.collecting.take() else {
            return;
        };
        let mut projections = Vec::new();
        let mut outcomes = Vec::new();
        let mut submitted = Vec::new();
        for item in round.items {
            let key = ExitIncarnation::new(item.flow.bytes());
            let revoked = resources
                .get(&item.flow)
                .map_or_else(|| stop.child_token(), |r| r.revoked.clone());
            let ready = !stop.is_cancelled() && item.scope.ensure_current().is_ok();
            if ready && let Some(tuple) = super::scope::tuple(item.tuple.clone()) {
                projections.push((
                    key,
                    tuple,
                    meshway_underlay::ExitCutoff {
                        at: item.scope.deadline(),
                        ms: item.scope.deadline_ms(),
                    },
                    ExitScope {
                        binding: item.flow.bytes().into(),
                        revoked: revoked.clone(),
                    },
                ));
                submitted.push(outcomes.len());
            }
            outcomes.push(Outcome {
                item,
                revoked,
                installed: false,
            });
        }
        let role = role.clone();
        self.running = Some(Box::pin(async move {
            let results = role.install_batch(projections).await;
            for (index, result) in submitted.into_iter().zip(results) {
                let key = ExitIncarnation::new(outcomes[index].item.flow.bytes());
                outcomes[index].installed = result.is_ok_and(|receipt| receipt.incarnation == key);
            }
            Done::Installed(outcomes)
        }));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Install,
    /// Renew, Retire or a barrier: strictly after everything before it.
    Ordered,
    Packet,
}

fn classify(work: &Work) -> (Kind, Option<FlowIncarnation>) {
    match work {
        Work::Projection(event) => match event.as_ref() {
            ExitProjection::Install { flow, .. } => (Kind::Install, Some(*flow)),
            ExitProjection::Renew { flow, .. } | ExitProjection::Retire { flow, .. } => {
                (Kind::Ordered, Some(*flow))
            }
        },
        Work::Packet(packet) => (Kind::Packet, packet.exit_flow()),
        Work::Barrier(_) => (Kind::Ordered, None),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Into the collecting round.
    Join,
    /// After the operation, in FIFO order.
    Hold,
    Run,
}

/// Work waiting for the operation, in its original order, with the flows it
/// names.
struct Held<K, W> {
    work: VecDeque<(Option<K>, W)>,
    flows: BTreeMap<K, usize>,
}

impl<K: Ord + Copy, W> Default for Held<K, W> {
    fn default() -> Self {
        Self {
            work: VecDeque::new(),
            flows: BTreeMap::new(),
        }
    }
}

impl<K: Ord + Copy, W> Held<K, W> {
    fn push(&mut self, flow: Option<K>, work: W) {
        if let Some(flow) = flow {
            *self.flows.entry(flow).or_default() += 1;
        }
        self.work.push_back((flow, work));
    }
    fn take(&mut self) -> VecDeque<W> {
        self.flows.clear();
        self.work.drain(..).map(|(_, work)| work).collect()
    }
    #[cfg(test)]
    fn pop(&mut self) -> Option<W> {
        let (flow, work) = self.work.pop_front()?;
        if let Some(flow) = flow
            && let Some(count) = self.flows.get_mut(&flow)
        {
            *count -= 1;
            if *count == 0 {
                self.flows.remove(&flow);
            }
        }
        Some(work)
    }
    fn len(&self) -> usize {
        self.work.len()
    }
    /// Where one item goes beside `operation` (its flows, whether its round
    /// still collects). Without an operation everything runs. With one, a packet waits only if its flow is in the operation
    /// or in held work; an installation joins a collecting round (a new
    /// flow, named by nothing held: other flows' renewals or retirements
    /// may follow it); the rest waits.
    fn route(
        &self,
        kind: Kind,
        flow: Option<&K>,
        operation: Option<(&BTreeSet<K>, bool)>,
    ) -> Route {
        let Some((flows, joinable)) = operation else {
            return Route::Run;
        };
        match kind {
            Kind::Install if joinable => Route::Join,
            Kind::Packet
                if !flow
                    .is_some_and(|flow| flows.contains(flow) || self.flows.contains_key(flow)) =>
            {
                Route::Run
            }
            _ => Route::Hold,
        }
    }
}

pub(super) async fn run(
    role: ExitRole,
    mut queue: mpsc::Receiver<Work>,
    failures: mpsc::Sender<FlowIncarnation>,
    health: watch::Sender<Health>,
    stop: CancellationToken,
) -> Result<(), Problem> {
    let mut resources: BTreeMap<FlowIncarnation, Resource> = BTreeMap::new();
    let mut failed = false;
    let mut ended = false;
    // A forward packet that died at this exit (TTL, MTU) never confirms; its
    // up to 3 s wait runs beside every other flow's work, not before it.
    let mut confirming = tokio::task::JoinSet::<Confirmation>::new();
    let mut operation: Option<Operation> = None;
    let mut held = Held::<FlowIncarnation, Work>::default();
    // Work held for an operation that has finished, routed again in its
    // order before the FIFO is read: a held burst of new flows shares the
    // next round, and what must follow that round is held for it.
    let mut replay = VecDeque::new();
    let mut closed = false;
    loop {
        if operation.is_none() && replay.is_empty() {
            replay = held.take();
        }
        let event = if let Some(work) = replay.pop_front() {
            Event::Work(work)
        } else if closed && operation.is_none() {
            break;
        } else {
            let due = operation.as_ref().and_then(Operation::due);
            let running = operation.as_ref().is_some_and(|op| op.running.is_some());
            tokio::select! {
                () = role.stopped(), if !ended && !stop.is_cancelled() => Event::Ended,
                Some(done) = confirming.join_next(), if !confirming.is_empty() => Event::Confirmed(done),
                done = async {
                    match operation.as_mut().and_then(|op| op.running.as_mut()) {
                        Some(running) => running.await,
                        None => std::future::pending().await,
                    }
                }, if running => Event::Done(done),
                () = async {
                    match due {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                }, if due.is_some() => Event::RoundDue,
                work = queue.recv(), if !closed && held.len() < super::CAPACITY => match work {
                    Some(work) => Event::Work(work),
                    None => Event::Closed,
                },
            }
        };
        let mut failure = None;
        let work = match event {
            Event::Work(work) => Some(work),
            Event::Confirmed(Ok((flow, original, confirmed))) => {
                if let Some(resource) = resources.get_mut(&flow) {
                    resource.confirming = false;
                    resource.confirmed |= confirmed;
                    if !confirmed && original.ensure_current().is_ok() {
                        resource.revoked.cancel();
                        failure = Some(flow);
                    }
                }
                None
            }
            Event::Confirmed(Err(_)) => {
                failed = true;
                None
            }
            Event::Done(done) => {
                if let Some(op) = operation.take() {
                    tracing::debug!(
                        operation = op.kind,
                        flows = op.flows.len(),
                        held = held.len(),
                        elapsed_us = op.started.elapsed().as_micros() as u64,
                        "Exit kernel operation answered"
                    );
                }
                match done {
                    // Each flow's own acknowledgement, whatever the others'.
                    Done::Installed(outcomes) => {
                        for Outcome {
                            item,
                            revoked,
                            installed,
                        } in outcomes
                        {
                            if item.ack.complete(installed).is_ok() {
                                resources.insert(
                                    item.flow,
                                    Resource {
                                        revoked,
                                        confirmed: false,
                                        confirming: false,
                                    },
                                );
                            } else {
                                revoked.cancel();
                                if failures.try_send(item.flow).is_err() {
                                    failed = true;
                                }
                            }
                        }
                    }
                    Done::Retired { ack, removed } => {
                        ack.complete(removed);
                        failed |= !removed;
                    }
                    Done::Renewed {
                        flow,
                        ack,
                        revoked,
                        renewed,
                    } => {
                        if ack.complete(renewed).is_err() {
                            revoked.cancel();
                            failure = Some(flow);
                        }
                    }
                }
                None
            }
            Event::RoundDue => {
                if let Some(op) = &mut operation {
                    op.submit(&role, &resources, &stop);
                }
                None
            }
            Event::Closed => {
                closed = true;
                None
            }
            Event::Ended => {
                ended = true;
                None
            }
        };
        // Route the work: the collecting round takes further installations,
        // work that must follow the operation waits for it, the rest runs now.
        let work = match work {
            Some(work) => {
                let (kind, flow) = classify(&work);
                let route = held.route(
                    kind,
                    flow.as_ref(),
                    operation.as_ref().map(|op| (&op.flows, op.joinable())),
                );
                match (route, &mut operation, work) {
                    (Route::Join, Some(op), Work::Projection(event)) => {
                        let ExitProjection::Install {
                            flow,
                            tuple,
                            scope,
                            ack,
                        } = *event
                        else {
                            unreachable!("only installations join")
                        };
                        op.join(Installing {
                            flow,
                            tuple,
                            scope,
                            ack,
                        });
                        if !op.joinable() {
                            op.submit(&role, &resources, &stop);
                        }
                        None
                    }
                    (Route::Hold, Some(_), work) => {
                        held.push(flow, work);
                        None
                    }
                    (_, _, work) => Some(work),
                }
            }
            None => None,
        };
        if let Some(work) = work {
            match work {
                Work::Barrier(reply) => {
                    let _ = reply.send(());
                }
                Work::Projection(event) => match *event {
                    ExitProjection::Install {
                        flow,
                        tuple,
                        scope,
                        ack,
                    } => {
                        operation = Some(Operation::round(Installing {
                            flow,
                            tuple,
                            scope,
                            ack,
                        }));
                    }
                    ExitProjection::Retire { flow, ack, .. } => {
                        if let Some(resource) = resources.remove(&flow) {
                            resource.revoked.cancel();
                        }
                        let role = role.clone();
                        operation = Some(Operation::running("retire", flow, async move {
                            let removed =
                                match role.retire_flow(ExitIncarnation::new(flow.bytes())).await {
                                    Ok(()) => true,
                                    // The Host may already have retired the whole role on
                                    // lease loss. Its verified all-flow cleanup also proves
                                    // this exact projection absent; an error never does.
                                    Err(_) if !role.is_active() => role.retire().await.is_ok(),
                                    Err(_) => false,
                                };
                            Done::Retired { ack, removed }
                        }));
                    }
                    ExitProjection::Renew {
                        flow,
                        tuple,
                        scope,
                        ack,
                    } => {
                        let key = ExitIncarnation::new(flow.bytes());
                        let revoked = resources
                            .get(&flow)
                            .map_or_else(|| stop.child_token(), |r| r.revoked.clone());
                        if stop.is_cancelled()
                            || scope.ensure_current().is_err()
                            || super::scope::tuple(tuple).is_none()
                        {
                            if ack.complete(false).is_err() {
                                revoked.cancel();
                                failure = Some(flow);
                            }
                        } else {
                            let deadline = meshway_underlay::ExitCutoff {
                                at: scope.deadline(),
                                ms: scope.deadline_ms(),
                            };
                            let kernel_scope = ExitScope {
                                binding: flow.bytes().into(),
                                revoked: revoked.clone(),
                            };
                            let role = role.clone();
                            operation = Some(Operation::running("renew", flow, async move {
                                let renewed = role
                                    .renew(key, deadline, kernel_scope)
                                    .await
                                    .is_ok_and(|receipt| receipt.incarnation == key);
                                Done::Renewed {
                                    flow,
                                    ack,
                                    revoked,
                                    renewed,
                                }
                            }));
                        }
                    }
                },
                Work::Packet(packet) => {
                    let packet = *packet;
                    if let Some(flow) = packet.exit_flow()
                        && let Some(resource) = resources.get_mut(&flow)
                        && !stop.is_cancelled()
                    {
                        let key = ExitIncarnation::new(flow.bytes());
                        let waiting = tokio::time::Instant::now();
                        let ready = packet.projection_ready().await;
                        if waiting.elapsed() >= Duration::from_millis(50) {
                            tracing::debug!(
                                waited_ms = waiting.elapsed().as_millis() as u64,
                                ready = ready.is_ok(),
                                "Exit packet waited for its projection"
                            );
                        }
                        if ready.is_err() {
                            continue;
                        }
                        let Ok(scope) = packet.exit_scope() else {
                            continue;
                        };
                        let original = scope.clone();
                        let sent = async {
                            let bytes = packet
                                .release(|action| match action {
                                    meshway_dataplane::Action::Exit { packet, .. } => {
                                        Some(packet.as_bytes().to_vec())
                                    }
                                    _ => None,
                                })
                                .map_err(|_| None)?
                                .ok_or(None)?;
                            role.send(
                                key,
                                bytes,
                                PacketGuard {
                                    scope,
                                    stop: stop.clone(),
                                },
                            )
                            .await
                            .map_err(Some)?;
                            if !resource.confirmed && !resource.confirming {
                                resource.confirming = true;
                                let role = role.clone();
                                let original = original.clone();
                                confirming.spawn(async move {
                                    let confirmed = matches!(
                                        role.confirm(key).await,
                                        Ok(CtConfirmation::Confirmed { incarnation, .. }) if incarnation == key
                                    );
                                    (flow, original, confirmed)
                                });
                            }
                            Ok::<_, Option<meshway_underlay::Error>>(())
                        }
                        .await;
                        // A full exit writer costs this packet only, like any
                        // full queue; other write failures end the flow.
                        if let Err(error) = sent
                            && !matches!(error, Some(meshway_underlay::Error::Capacity))
                            && original.ensure_current().is_ok()
                        {
                            resource.revoked.cancel();
                            failure = Some(flow);
                        }
                    }
                }
            }
        }
        if let Some(flow) = failure {
            tracing::debug!(?flow, "Exit flow failed by its own projection");
        }
        if let Some(flow) = failure
            && failures.try_send(flow).is_err()
        {
            failed = true;
        }
        if failed {
            stop.cancel();
        }
        let next = Health {
            flows: resources
                .values()
                .filter(|resource| !resource.revoked.is_cancelled())
                .count() as u32,
            failed,
            ended,
        };
        health.send_if_modified(|current| {
            if *current == next {
                return false;
            }
            *current = next;
            true
        });
    }
    stop.cancel();
    confirming.abort_all();
    while confirming.join_next().await.is_some() {}
    let retired = role.retire().await.map_err(|_| Problem::Underlay);
    health.send_replace(Health {
        flows: 0,
        failed: failed || retired.is_err(),
        ended,
    });
    retired.and(if failed {
        Err(Problem::Underlay)
    } else {
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{Held, Kind, Route};
    use std::collections::BTreeSet;

    #[test]
    fn held_work_replays_in_order_and_a_held_burst_shares_one_round() {
        let mut held = Held::<u8, &str>::default();
        // Held behind a retirement: a traceroute's new flows 2 and 3 with
        // their first packets, then flow 9's renewal and packet.
        for (flow, item) in [
            (2, "i2"),
            (2, "p2"),
            (3, "i3"),
            (3, "p3"),
            (9, "r9"),
            (9, "p9"),
        ] {
            held.push(Some(flow), item);
        }
        let replay = held.take();
        assert_eq!(held.len(), 0);
        // Without an operation the first item runs: i2 opens a round.
        assert_eq!(held.route(Kind::Install, Some(&2), None), Route::Run);
        let mut round = BTreeSet::from([2]);
        let mut routes = Vec::new();
        for (kind, flow, item) in [
            (Kind::Packet, 2, replay[1]),
            (Kind::Install, 3, replay[2]),
            (Kind::Packet, 3, replay[3]),
            (Kind::Ordered, 9, replay[4]),
            (Kind::Packet, 9, replay[5]),
        ] {
            let route = held.route(kind, Some(&flow), Some((&round, true)));
            match route {
                Route::Join => {
                    round.insert(flow);
                }
                Route::Hold => held.push(Some(flow), item),
                Route::Run => {}
            }
            routes.push(route);
        }
        assert_eq!(
            routes,
            [
                Route::Hold,
                Route::Join,
                Route::Hold,
                Route::Hold,
                Route::Hold
            ]
        );
        // What waits for the new round keeps its order.
        assert_eq!(held.take(), ["p2", "p3", "r9", "p9"]);
    }

    #[test]
    fn an_operation_holds_only_its_own_flows_and_later_ordered_work() {
        let mut held = Held::<u8, u8>::default();
        let round = BTreeSet::from([1]);
        // An installed flow's packet passes a round; the round's own waits.
        assert_eq!(
            held.route(Kind::Packet, Some(&2), Some((&round, true))),
            Route::Run
        );
        assert_eq!(
            held.route(Kind::Packet, Some(&1), Some((&round, true))),
            Route::Hold
        );
        held.push(Some(1), 0);
        // A renewal or retirement waits for the operation ...
        assert_eq!(
            held.route(Kind::Ordered, Some(&2), Some((&round, true))),
            Route::Hold
        );
        held.push(Some(2), 1);
        // ... and so do that flow's packets, while other flows pass.
        assert_eq!(
            held.route(Kind::Packet, Some(&2), Some((&round, true))),
            Route::Hold
        );
        assert_eq!(
            held.route(Kind::Packet, Some(&4), Some((&round, false))),
            Route::Run
        );
        // New flows keep joining the collecting round (one burst, one round).
        assert_eq!(
            held.route(Kind::Install, Some(&3), Some((&round, true))),
            Route::Join
        );
        assert_eq!(
            held.route(Kind::Install, Some(&3), Some((&round, false))),
            Route::Hold
        );
        assert_eq!(
            held.route(Kind::Ordered, None, Some((&round, false))),
            Route::Hold
        );
        assert_eq!(held.pop(), Some(0));
        assert_eq!(held.pop(), Some(1));
        assert_eq!(held.len(), 0);
        assert_eq!(
            held.route(Kind::Packet, Some(&2), Some((&round, false))),
            Route::Run
        );
    }
}
