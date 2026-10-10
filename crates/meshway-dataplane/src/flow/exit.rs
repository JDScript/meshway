use super::*;
use crate::{
    exit::{
        self, ExitProjection, ExitTuple, FlowIncarnation, Gate, ProjectionScope, RetirementAck,
    },
    secure::OutputBinding,
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

    /// Activity renewal of an exit terminal's kernel projection. A gate of
    /// replaced content (its scope is no longer current) is renewed with the
    /// current guard on the next admitted packet; no successor is precomputed.
    pub fn prepare_projection(
        &self,
        prepared: &mut Prepared,
        guard: &OutputBinding,
        now: (Instant, u64),
    ) -> Result<(), Error> {
        let Some(exit) = &mut prepared.flow.exit else {
            return Ok(());
        };
        if !exit.live(now.0, now.1) {
            return Err(Error::Expired);
        }
        if !exit.terminal {
            return Ok(());
        }
        let renew_after = self.limits.idle_timeout / 2;
        // Compare effective bounds of the circuit and the flow.
        let bound = guard
            .expiry()
            .map_or(prepared.cap, |cap| cap.min(prepared.cap));
        let renew = exit.gate.as_ref().is_none_or(|gate| {
            gate.scope.ensure_current().is_err()
                || (gate.ready().is_ok()
                    && (bound.ms > gate.scope.deadline_ms()
                        || (prepared
                            .cap
                            .at
                            .saturating_duration_since(gate.scope.deadline())
                            >= renew_after / 2
                            && gate.scope.deadline().saturating_duration_since(now.0)
                                <= renew_after)))
        });
        if renew {
            // Fence against the actual kept flow bound, not merely the
            // older (potentially longer) installed projection.
            let previous_deadline = prepared.previous_deadline;
            exit.renew = exit.gate.is_some();
            let scope =
                ProjectionScope::new(guard.for_projection(prepared.cap, exit.cancellation.clone()));
            exit.gate = Some(Gate::new(
                scope,
                exit.cancellation.clone(),
                self.pending_projections.clone(),
                self.exit_limits,
                now.0,
                previous_deadline,
            )?);
        }
        // A packet can refresh idle only within an installed or pending-ACK
        // projection. Never retain protocol state beyond a gate which could
        // expire before the next activity and then revive its conntrack tuple.
        if let Some(gate) = &exit.gate {
            let cap = Cap {
                at: gate.scope.deadline(),
                ms: gate.scope.deadline_ms(),
            };
            prepared.cap = prepared.cap.min(cap);
            prepared.flow.cap = prepared.cap;
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

    pub fn fail_exit_projection(&mut self, incarnation: &FlowIncarnation) -> bool {
        let key = self.entries.iter().find_map(|(key, flow)| {
            flow.exit
                .as_ref()
                .is_some_and(|exit| exit.incarnation == *incarnation)
                .then(|| key.clone())
        });
        let Some(key) = key else {
            return false;
        };
        self.remove(&key);
        true
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
}
