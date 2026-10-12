//! Stateless cryptographic jobs separated from the unique circuit/flow owner.
//! Jobs are opaque and move-only. Workers never query authority; every result
//! retains its original guard and is checked by the owner or final I/O adapter.
use super::{
    Carrier, Direction, Entry, Error, Hop, OutputBinding, Packet, PacketBuffer, PendingFrame,
    Received, SecureNode, State, View, authorization,
    codec::{self, Frame, Kind},
    crypto,
    session::MacKey,
};
use meshway_model::NodeId;
use std::sync::Arc;
mod credit;
mod pool;
pub(crate) mod timing;
pub use credit::{
    Credit, JUMBO_CAPACITY, JUMBO_STORAGE, MTU_STORAGE, STREAM_BYTES, STREAM_CAPACITY,
};
pub use pool::{Job, Output, Pool, Ready};

/// A committed nonce and flow admission; abandoning this job never rolls them back.
pub struct SendJob {
    pub(super) frame: Frame,
    pub(super) noise: Arc<crypto::Transport>,
    pub(super) mac: MacKey,
    pub(super) next: NodeId,
    pub(super) carrier: Carrier,
    pub(super) guard: OutputBinding,
}

impl std::fmt::Debug for SendJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendJob")
            .field("next", &self.next)
            .field("length", &self.frame.body.len())
            .finish_non_exhaustive()
    }
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
    guard: OutputBinding,
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
    guard: OutputBinding,
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
    /// Data only. Handshakes and probes use the independent synchronous receive path.
    pub fn prepare_receive(
        &mut self,
        incoming: &Carrier,
        bytes: PacketBuffer,
    ) -> Result<RecvJob, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.prepare_receive");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
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
        let selected = authorization::selected(&circuit.context, circuit.selected.as_deref())?;
        let (previous, mac) = circuit.session.prepare_verify(selected, &frame, incoming)?;
        authorization::incoming(
            &mut self.engine.decisions,
            &reading,
            selected,
            previous,
            incoming,
        )?;
        let guard = circuit.guard(false);
        let target = match &circuit.state {
            State::Endpoint(noise) => ReceiveTarget::Endpoint(noise.clone()),
            State::RelayEstablished => {
                let path = &circuit.context.path;
                let position = path
                    .iter()
                    .position(|node| *node == reading.node())
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
                authorization::outgoing(Some(selected), next, &carrier)?;
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

    /// Rechecks the original circuit lifetime, owner/key identity and replay
    /// window before any endpoint flow mutation, on the current content. A
    /// circuit the content no longer admits was retired, which ends its jobs.
    /// Duplicate jobs may verify concurrently but only the first successful
    /// owner commit can consume their sequence.
    pub fn commit_receive(&mut self, opened: Opened) -> Result<Received, Error> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("secure.commit_receive");
        let _profile = PROFILE.enter();
        let desired = self.desired.clone();
        let reading = View::of(&desired);
        opened.guard.check_at(&reading)?;
        let id = opened.circuit;
        let circuit = self.circuits.get_mut(&id).ok_or(Error::State)?;
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
                let grant = circuit.context.grant.flow().ok_or(Error::Policy)?;
                if opened.bytes.is_empty() {
                    // A passive keepalive carries no packet and admits no flow.
                    opened.guard.finish()?;
                    circuit.receive[index].commit(sequence);
                    circuit.received(reading.instant);
                    return Ok(Received::Keepalive { circuit: id });
                }
                Self::check_outgoing_carrier(&self.carriers, circuit, opened.reply, &reading)?;
                let path = &circuit.context.path;
                let entry = Entry {
                    grant,
                    path,
                    origin: path[0],
                    sender: if opened.reply {
                        *path.last().ok_or(Error::Context)?
                    } else {
                        path[0]
                    },
                    previous: Some(opened.previous),
                };
                let packet = Packet::parse(opened.bytes).map_err(|_| Error::Packet)?;
                let mut route = self
                    .engine
                    .admit_circuit_facts(&entry, packet.facts(), &reading, id, &opened.guard)
                    .map_err(|error| circuit.engine_rejected("receive_endpoint", error))?;
                route
                    .constrain(&reading, &opened.guard.life)
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
            circuit.received(reading.instant);
            if matches!(result, Received::Packet(_)) {
                circuit.owe_keepalive(reading.instant);
            }
            Ok(result)
        })();
        circuit.finish_operation("receive", result)
    }
}
