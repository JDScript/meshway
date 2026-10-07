//! Stateless cryptographic jobs separated from the unique circuit/flow owner.
//! Jobs are opaque and move-only. Workers never query authority; every result
//! retains its original guard and is checked by the owner or final I/O adapter.
use super::{
    AuthenticatedPeer, Carrier, CircuitAuthorization, Direction, Error, Guard, Hop, Ingress,
    Packet, PacketBuffer, PendingFrame, Received, SecureNode, State,
    codec::{self, Frame, Kind},
    crypto,
    session::MacKey,
};
use meshway_model::NodeId;
use std::sync::Arc;
mod pool;
pub(crate) mod timing;
pub use pool::{Credit, Job, Output, Pool, Ready};

/// A committed nonce and flow admission; abandoning this job never rolls them back.
pub struct SendJob {
    pub(super) frame: Frame,
    pub(super) noise: Arc<crypto::Transport>,
    pub(super) mac: MacKey,
    pub(super) next: NodeId,
    pub(super) carrier: Carrier,
    pub(super) guard: Guard,
}

impl SendJob {
    pub fn circuit(&self) -> [u8; 32] {
        self.frame.context.circuit
    }
}

/// No replay state is reserved or committed until the verified result returns.
pub struct RecvJob {
    frame: Frame,
    mac: MacKey,
    target: ReceiveTarget,
    previous: NodeId,
    guard: Guard,
}

impl RecvJob {
    pub fn circuit(&self) -> [u8; 32] {
        self.frame.context.circuit
    }
}

enum ReceiveTarget {
    Endpoint(Arc<crypto::Transport>),
    Relay {
        next: NodeId,
        carrier: Carrier,
        mac: MacKey,
    },
}

/// Authenticated worker output, still uncommitted and inaccessible as plaintext.
pub struct Opened {
    bytes: PacketBuffer,
    circuit: [u8; 32],
    sequence: u64,
    reply: bool,
    mac: MacKey,
    previous: NodeId,
    relay: Option<(NodeId, Carrier)>,
    guard: Guard,
}

/// Inline worker implementation, also used by the bounded pool.
pub fn seal(mut job: SendJob) -> Result<PendingFrame, Error> {
    static PROFILE: meshway_logging::profile::Metric =
        meshway_logging::profile::Metric::new("crypto.encrypt");
    {
        let _profile = PROFILE.enter();
        job.noise.encrypt(job.frame.sequence, &mut job.frame.body)?;
    }
    job.mac.append(&mut job.frame)?;
    Ok(PendingFrame {
        circuit: job.frame.context.circuit,
        bytes: job.frame.encode_owned()?,
        next: job.next,
        carrier: job.carrier,
        guard: job.guard,
    })
}

/// Verify before decrypting or replacing the adjacent tag. No authority reads.
pub fn open(mut job: RecvJob) -> Result<Opened, Error> {
    job.mac.verify(&job.frame)?;
    let relay = match job.target {
        ReceiveTarget::Endpoint(noise) => {
            static PROFILE: meshway_logging::profile::Metric =
                meshway_logging::profile::Metric::new("crypto.decrypt");
            let _profile = PROFILE.enter();
            noise.decrypt(job.frame.sequence, &mut job.frame.body)?;
            None
        }
        ReceiveTarget::Relay { next, carrier, mac } => {
            mac.append(&mut job.frame)?;
            Some((next, carrier))
        }
    };
    Ok(Opened {
        circuit: job.frame.context.circuit,
        sequence: job.frame.sequence,
        reply: job.frame.reply,
        bytes: if relay.is_some() {
            job.frame.encode_owned()?
        } else {
            job.frame.body
        },
        mac: job.mac,
        previous: job.previous,
        relay,
        guard: job.guard,
    })
}

impl SecureNode {
    pub fn prepare_send(
        &mut self,
        circuit: [u8; 32],
        bytes: PacketBuffer,
    ) -> Result<SendJob, Error> {
        let snapshot = self.identity.snapshot().clone();
        let reading = snapshot.reading().map_err(|_| Error::Authority)?;
        self.prepare_send_at(
            circuit,
            Packet::parse(bytes).map_err(|_| Error::Packet)?,
            &reading,
        )
    }

    /// Data only. Handshakes and probes use the independent synchronous receive path.
    pub fn prepare_receive(
        &mut self,
        incoming: &Carrier,
        bytes: PacketBuffer,
    ) -> Result<RecvJob, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.prepare_receive");
        let _profile = PROFILE.enter();
        let snapshot = self.identity.snapshot().clone();
        let reading = snapshot.reading().map_err(|_| Error::Authority)?;
        self.maintain_circuits(&reading, false)?;
        let id = codec::session_id(&bytes)?.ok_or(Error::Wire)?;
        let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
        if let Err(error) = circuit.check_at(&reading) {
            circuit.retire_for("prepare_receive", "deadline_or_cancel");
            return Err(error);
        }
        let frame = Frame::decode_owned(bytes, Some(&circuit.context))?;
        if frame.kind != Kind::Data {
            return Err(Error::Wire);
        }
        circuit.receive[usize::from(frame.reply)].check(frame.sequence)?;
        let (previous, mac) =
            circuit
                .session
                .prepare_verify(reading.configuration, &frame, incoming)?;
        let mut guard = circuit.guard(&snapshot, false);
        guard.section = Some(self.output_epoch.clone());
        let target = match &circuit.state {
            State::Endpoint(noise) => ReceiveTarget::Endpoint(noise.clone()),
            State::RelayEstablished => {
                let path = &circuit.context.path;
                let position = path
                    .iter()
                    .position(|node| *node == reading.configuration.node_id)
                    .ok_or(Error::Context)?;
                let next = if frame.reply {
                    position.checked_sub(1)
                } else {
                    position.checked_add(1)
                }
                .and_then(|position| path.get(position))
                .copied()
                .ok_or(Error::Context)?;
                let carrier = self.carriers.get(&next).ok_or(Error::Unsupported)?.clone();
                meshway_policy::admission::admit_circuit_carrier(
                    reading.configuration,
                    circuit.context.authorization,
                    path,
                    next,
                    &carrier,
                )
                .map_err(|_| Error::Policy)?;
                let mac = circuit.session.prepare_append(&frame, &carrier)?;
                ReceiveTarget::Relay { next, carrier, mac }
            }
            _ => return Err(Error::State),
        };
        guard.finish()?;
        Ok(RecvJob {
            frame,
            mac,
            target,
            previous,
            guard,
        })
    }

    /// Rechecks the original scope, owner/key identity and replay window before
    /// any endpoint flow mutation. Duplicate jobs may verify concurrently but
    /// only the first successful owner commit can consume their sequence.
    pub fn commit_receive(&mut self, opened: Opened) -> Result<Received, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.commit_receive");
        let _profile = PROFILE.enter();
        let snapshot = opened.guard.snapshot.clone();
        let reading = snapshot.reading().map_err(|_| Error::Authority)?;
        opened.guard.check_at(&reading)?;
        if !snapshot.same_execution(self.identity.snapshot()) {
            return Err(Error::Authority);
        }
        let id = opened.circuit;
        let mut circuit = self.circuits.remove(&id).ok_or(Error::State)?;
        let result = (|| {
            circuit.check_at(&reading)?;
            // The immutable MAC-key allocation identifies this exact owner's
            // established context; jobs cannot be constructed or rebound by callers.
            if !circuit.session.owns(&opened.mac) {
                return Err(Error::Context);
            }
            let index = usize::from(opened.reply);
            circuit.receive[index].check(opened.sequence)?;
            let sequence = opened.sequence;
            let result = if let Some((next, carrier)) = opened.relay {
                if !matches!(circuit.state, State::RelayEstablished) {
                    return Err(Error::State);
                }
                if self.carriers.get(&next) != Some(&carrier) {
                    return Err(Error::Context);
                }
                Received::Forward(PendingFrame {
                    bytes: opened.bytes,
                    next,
                    carrier,
                    circuit: id,
                    guard: opened.guard.clone(),
                })
            } else {
                if !matches!(circuit.state, State::Endpoint(_)) {
                    return Err(Error::State);
                }
                let CircuitAuthorization::Business(demand) = circuit.context.authorization else {
                    return Err(Error::Policy);
                };
                self.check_outgoing_carrier(&circuit.context, opened.reply, &reading)?;
                let path = &circuit.context.path;
                let ingress = Ingress::Peer {
                    demand,
                    path: path.clone(),
                    identity: AuthenticatedPeer {
                        origin: path[0],
                        sender: if opened.reply {
                            *path.last().ok_or(Error::Context)?
                        } else {
                            path[0]
                        },
                        previous: opened.previous,
                    },
                };
                let packet = Packet::parse(opened.bytes).map_err(|_| Error::Packet)?;
                let mut route = self
                    .engine
                    .admit_circuit_facts(ingress, packet.facts(), &reading, id, &opened.guard)
                    .map_err(|error| circuit.engine_rejected("receive_endpoint", error))?;
                route
                    .constrain(
                        &reading,
                        opened.guard.expires_at,
                        opened.guard.expires_at_ms,
                        opened.guard.cancellation.clone(),
                    )
                    .map_err(|_| Error::Authority)?;
                if (route.route().direction == Direction::Reply) != opened.reply {
                    return Err(Error::Context);
                }
                if !matches!(route.hop(), Hop::Deliver | Hop::Exit) {
                    return Err(Error::State);
                }
                Received::Packet(route.attach(packet).map_err(|_| Error::Packet)?)
            };
            opened.guard.finish()?;
            circuit.receive[index].commit(sequence);
            Ok(result)
        })();
        let result = circuit.finish_operation(self.identity.snapshot(), "receive", result);
        self.circuits.insert(id, circuit);
        result
    }
}
