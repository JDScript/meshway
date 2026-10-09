use super::*;
use crate::{
    exit::{
        self, ExitProjection, ExitTuple, FlowIncarnation, Gate, ProjectionScope, RetirementAck,
    },
    secure::Guard,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(super) struct ExitState {
    incarnation: FlowIncarnation,
    tuple: ExitTuple,
    terminal: bool,
    pub cancellation: CancellationToken,
    pub gate: Option<Arc<Gate>>,
    successor: Option<ProjectionScope>,
    renew: bool,
}

#[derive(Clone)]
pub(super) struct Tombstone {
    state: ExitState,
    circuit: Option<[u8; 32]>,
    claimed: Arc<AtomicBool>,
    quarantine_until: Arc<Mutex<Option<Instant>>>,
}

impl ExitState {
    pub fn cancel(&self) {
        self.cancellation.cancel();
        if let Some(gate) = &self.gate {
            gate.retire();
        }
    }
    fn new(tuple: &Tuple, terminal: bool) -> Result<Self, Error> {
        let (IpAddr::V4(source), IpAddr::V4(destination)) = (tuple.source, tuple.destination)
        else {
            return Err(Error::Flow);
        };
        let mut id = [0; 32];
        getrandom::fill(&mut id).map_err(|_| Error::Flow)?;
        Ok(Self {
            incarnation: FlowIncarnation(id),
            tuple: ExitTuple {
                source,
                destination,
                source_port: tuple.source_port.map(Port::get),
                destination_port: tuple.destination_port.map(Port::get),
                protocol: match tuple.protocol {
                    6 => BusinessProtocol::Tcp,
                    17 => BusinessProtocol::Udp,
                    _ => BusinessProtocol::IcmpEcho,
                },
                echo: tuple.echo,
            },
            terminal,
            cancellation: CancellationToken::new(),
            gate: None,
            successor: None,
            renew: false,
        })
    }
    pub fn live(&self, now: Instant, now_ms: u64) -> bool {
        !self.cancellation.is_cancelled()
            && self
                .gate
                .as_ref()
                .is_none_or(|gate| !gate.failed(now, now_ms))
    }
}

impl FlowTable {
    pub fn set_exit_limits(&mut self, limits: exit::Limits) -> Result<(), Error> {
        if !self.entries.is_empty() || !self.tombstones.is_empty() {
            return Err(Error::Limits);
        }
        self.exit_limits = limits.validate()?;
        Ok(())
    }

    pub(super) fn exit_state(
        &self,
        key: &Key,
        existing: Option<&Flow>,
        is_exit: bool,
    ) -> Result<Option<ExitState>, Error> {
        if !is_exit {
            return Ok(None);
        }
        if let Some(flow) = existing {
            return Ok(flow.exit.clone());
        }
        if self.tombstones.contains_key(&key.tuple)
            || self.tombstones.contains_key(&key.tuple.reversed())
        {
            return Err(Error::Flow);
        }
        let live = self.exit_count;
        if live >= self.exit_limits.max_flows
            || live + self.tombstones.len() >= self.exit_limits.max_tombstones
        {
            return Err(Error::Capacity);
        }
        Ok(Some(ExitState::new(
            &key.tuple,
            key.role == Role::Terminal,
        )?))
    }

    pub fn prepare_projection(
        &self,
        prepared: &mut Prepared,
        guard: &Guard,
        reading: &meshway_authority::Reading<'_>,
    ) -> Result<(), Error> {
        let Some(exit) = &mut prepared.flow.exit else {
            return Ok(());
        };
        if !exit.live(reading.instant, reading.now_ms) {
            return Err(Error::Expired);
        }
        if !exit.terminal {
            return Ok(());
        }
        let renew_after = self.limits.idle_timeout / 2;
        let renew = exit.gate.as_ref().is_none_or(|gate| {
            gate.scope.ensure_current().is_err()
                || (gate.ready().is_ok()
                    && (prepared.permit_expires_at_ms.min(guard.expires_at_ms)
                        > gate.scope.deadline_ms()
                        || (prepared
                            .expires_at
                            .saturating_duration_since(gate.scope.deadline())
                            >= renew_after / 2
                            && gate
                                .scope
                                .deadline()
                                .saturating_duration_since(reading.instant)
                                <= renew_after)))
        });
        if renew {
            // Reconciliation can shorten retained flow bounds before a lazy
            // kernel renewal is materialized. Fence against that actual flow,
            // not merely the older (potentially longer) installed projection.
            let previous_deadline = prepared.previous_deadline;
            exit.renew = exit.gate.is_some();
            let scope = ProjectionScope::new(guard.for_projection(
                prepared.expires_at,
                prepared.permit_expires_at_ms,
                exit.cancellation.clone(),
            ));
            exit.gate = Some(Gate::new(
                scope,
                exit.cancellation.clone(),
                self.pending_projections.clone(),
                self.exit_limits,
                reading.instant,
                previous_deadline,
            )?);
            exit.successor = None;
        }
        // A packet can refresh idle only within an installed or pending-ACK
        // projection. Never retain protocol state beyond a gate which could
        // expire before the next activity and then revive its conntrack tuple.
        if let Some(gate) = &exit.gate {
            prepared.expires_at = prepared.expires_at.min(gate.scope.deadline());
            prepared.permit_expires_at_ms =
                prepared.permit_expires_at_ms.min(gate.scope.deadline_ms());
            prepared.flow.expires_at = prepared.expires_at;
            prepared.flow.permit_expires_at_ms = prepared.permit_expires_at_ms;
        }
        Ok(())
    }

    pub(super) fn retire_exit(&mut self, key: &Key, flow: &Flow) {
        let Some(state) = &flow.exit else {
            return;
        };
        // Cancellation precedes publishing Retire and removing flow/cipher state.
        state.cancel();
        self.retired_circuits_pending |= flow.circuit.is_some();
        self.tombstones
            .entry(key.tuple.clone())
            .or_insert_with(|| Tombstone {
                state: state.clone(),
                circuit: flow.circuit,
                claimed: Arc::new(AtomicBool::new(false)),
                quarantine_until: Arc::new(Mutex::new(
                    (!state.terminal).then(|| Instant::now() + self.exit_limits.tombstone_timeout),
                )),
            });
    }

    pub(super) fn expire_exits(&mut self, now: Instant, now_ms: u64) {
        if !self.has_exit_work() {
            return;
        }
        let dead: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, flow)| {
                flow.exit
                    .as_ref()
                    .is_some_and(|exit| !exit.live(now, now_ms))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in dead {
            self.remove(&key);
        }
        self.tombstones.retain(|_, tombstone| {
            !tombstone
                .quarantine_until
                .lock()
                .is_ok_and(|until| until.is_some_and(|deadline| now >= deadline))
        });
    }

    /// Commit reconciliation side effects only after the caller's final fence.
    pub fn replace(&mut self, mut next: Self) {
        let retained: std::collections::BTreeSet<_> = next
            .entries
            .values()
            .filter_map(|flow| flow.exit.as_ref().map(|exit| exit.incarnation))
            .collect();
        let retired: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, flow)| {
                flow.exit
                    .as_ref()
                    .is_some_and(|exit| !retained.contains(&exit.incarnation))
            })
            .map(|(key, flow)| (key.clone(), flow.clone()))
            .collect();
        for (key, flow) in retired {
            self.retire_exit(&key, &flow);
        }
        next.tombstones = std::mem::take(&mut self.tombstones);
        next.retired_circuits_pending = self.retired_circuits_pending;
        next.pending_projections = self.pending_projections.clone();
        *self = next;
    }

    pub fn project_successor(
        &mut self,
        snapshot: &meshway_authority::ExecutionSnapshot,
        reading: &meshway_authority::Reading<'_>,
    ) {
        for flow in self.entries.values_mut() {
            let Some(exit) = &mut flow.exit else {
                continue;
            };
            let Some(gate) = &exit.gate else {
                continue;
            };
            let scope = ProjectionScope::new(
                gate.scope.0.successor_projection(
                    snapshot.clone(),
                    flow.expires_at.min(reading.expires_at),
                    flow.permit_expires_at_ms
                        .min(flow.guard_expires_at_ms)
                        .min(reading.expires_at_ms),
                ),
            );
            // Keep the renewal intent on the flow itself. Materialize at most
            // max_pending commands as the host drains/ACKs them; a large live
            // table does not need a second unbounded command queue.
            exit.successor = Some(scope);
            exit.renew = true;
        }
    }

    pub fn next_exit_projection(&mut self) -> Option<ExitProjection> {
        // Retire always drains before new positive operations and is available
        // even when the execution snapshot has expired.
        for tombstone in self.tombstones.values() {
            if tombstone.state.terminal && !tombstone.claimed.load(Ordering::Acquire) {
                if self
                    .pending_projections
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                        (n < self.exit_limits.max_pending).then_some(n + 1)
                    })
                    .is_err()
                {
                    // Already-reserved positive operations can still be handed
                    // out below; their ACKs free slots for prioritized cleanup.
                    break;
                }
                tombstone.claimed.store(true, Ordering::Release);
                return Some(ExitProjection::Retire {
                    flow: tombstone.state.incarnation,
                    tuple: tombstone.state.tuple.clone(),
                    scope: tombstone
                        .state
                        .gate
                        .as_ref()
                        .expect("terminal projection")
                        .scope
                        .clone(),
                    ack: RetirementAck {
                        quarantine_until: tombstone.quarantine_until.clone(),
                        quarantine: self.exit_limits.tombstone_timeout,
                        budget: self.pending_projections.clone(),
                    },
                });
            }
        }
        for flow in self.entries.values_mut() {
            let Some(state) = &mut flow.exit else {
                continue;
            };
            if let Some(scope) = &state.successor {
                let Ok(gate) = Gate::new(
                    scope.clone(),
                    state.cancellation.clone(),
                    self.pending_projections.clone(),
                    self.exit_limits,
                    Instant::now(),
                    Some((
                        flow.expires_at,
                        flow.permit_expires_at_ms.min(flow.guard_expires_at_ms),
                    )),
                ) else {
                    continue;
                };
                state.gate = Some(gate);
                state.successor = None;
            }
            let Some(gate) = &state.gate else {
                continue;
            };
            let Some(ack) = gate.claim() else {
                continue;
            };
            return Some(if state.renew {
                ExitProjection::Renew {
                    flow: state.incarnation,
                    tuple: state.tuple.clone(),
                    scope: gate.scope.clone(),
                    ack,
                }
            } else {
                ExitProjection::Install {
                    flow: state.incarnation,
                    tuple: state.tuple.clone(),
                    scope: gate.scope.clone(),
                    ack,
                }
            });
        }
        None
    }

    pub fn retired_exit_circuits(&mut self) -> std::collections::BTreeSet<[u8; 32]> {
        if !std::mem::take(&mut self.retired_circuits_pending) {
            return Default::default();
        }
        self.tombstones
            .values_mut()
            .filter_map(|tombstone| tombstone.circuit.take())
            .collect()
    }
    pub fn has_exit_work(&self) -> bool {
        self.exit_count != 0 || !self.tombstones.is_empty()
    }

    pub fn has_exit_circuit(&self, id: [u8; 32]) -> bool {
        self.entries
            .values()
            .any(|flow| flow.exit.is_some() && flow.circuit == Some(id))
    }

    pub fn retire_exit_circuit(&mut self, id: [u8; 32]) {
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, flow)| flow.exit.is_some() && flow.circuit == Some(id))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }

    pub fn retire_exits(&mut self) {
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, flow)| flow.exit.is_some())
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            self.remove(&key);
        }
    }
}

impl Prepared {
    pub fn exit_live_at(&self, now: Instant, now_ms: u64) -> bool {
        self.flow
            .exit
            .as_ref()
            .is_none_or(|exit| exit.live(now, now_ms))
    }
    pub fn exit_incarnation(&self) -> Option<FlowIncarnation> {
        self.flow.exit.as_ref().map(|exit| exit.incarnation)
    }
    pub fn exit_guards(&self) -> (Option<CancellationToken>, Option<Arc<Gate>>) {
        self.flow.exit.as_ref().map_or((None, None), |exit| {
            (Some(exit.cancellation.clone()), exit.gate.clone())
        })
    }
    pub fn is_exit(&self) -> bool {
        self.flow.exit.is_some()
    }
    pub fn hop_is_exit(&self) -> bool {
        self.hop == Hop::Exit
    }
}
