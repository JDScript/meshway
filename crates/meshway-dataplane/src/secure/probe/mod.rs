//! Fixed-size diagnostic messages over the sole circuit/Noise/proof owner.
//! Request correlations are separate from IP flows and retain their original
//! execution guard across circuit continuation. Reading never renews a request.
use super::*;
use codec::ProbeMetadata;
use meshway_model::{Digest, EventId, ObservationId};

const MAX_PENDING: usize = 128;
const INTERVAL: Duration = Duration::from_secs(1);
const MAX_LIFETIME: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
enum RateRole {
    Source,
    Remote,
}

struct Rate {
    role: RateRole,
    /// Virtual instant when all spent tokens have replenished. Source has one
    /// token; remote has two to tolerate compression of lawful source cadence
    /// by path jitter. Both replenish one token per second across all CIDs.
    full_at: Instant,
}

struct Association {
    metadata: ProbeMetadata,
    challenge: Option<Digest>,
    guard: Guard,
    completed: bool,
}

#[derive(Default)]
pub(super) struct Probes {
    pending: BTreeMap<([u8; 32], EventId), Association>,
    rates: BTreeMap<ObservationId, Rate>,
}
impl Probes {
    pub(super) fn maintain(&mut self, reading: &Reading<'_>) {
        self.pending
            .retain(|_, request| request.guard.ensure_current().is_ok());
        self.maintain_rates(reading.instant);
    }

    fn maintain_rates(&mut self, now: Instant) {
        // Forget only fully replenished entries: expiry cannot mint tokens or
        // reset the observation's debt while any of its budget remains spent.
        self.rates.retain(|_, rate| now < rate.full_at);
    }

    fn admit_rate(
        &mut self,
        observation: ObservationId,
        role: RateRole,
        now: Instant,
    ) -> Result<(), Error> {
        self.maintain_rates(now);
        let full_at = if let Some(rate) = self.rates.get(&observation) {
            if rate.role != role {
                return Err(Error::Policy);
            }
            let tolerance = match role {
                RateRole::Source => Duration::ZERO,
                RateRole::Remote => INTERVAL,
            };
            if rate.full_at.saturating_duration_since(now) > tolerance {
                return Err(Error::Capacity);
            }
            rate.full_at.max(now)
        } else {
            if self.rates.len() >= MAX_PENDING {
                return Err(Error::Capacity);
            }
            now
        };
        self.rates.insert(
            observation,
            Rate {
                role,
                full_at: full_at + INTERVAL,
            },
        );
        Ok(())
    }

    fn admit(
        &mut self,
        observation: ObservationId,
        circuit: [u8; 32],
        measurement: EventId,
        role: RateRole,
        reading: &Reading<'_>,
    ) -> Result<(), Error> {
        self.maintain(reading);
        if self.pending.contains_key(&(circuit, measurement)) {
            return Err(Error::Replay);
        }
        if self.pending.len() >= MAX_PENDING {
            return Err(Error::Capacity);
        }
        // Budget debt starts at actual admission. Using a stale entry instant
        // here would replenish tokens early after a slow synchronous section.
        self.admit_rate(observation, role, Instant::now())
    }
}

fn encrypt(
    noise: &crypto::Transport,
    sequence: u64,
    challenge: Digest,
) -> Result<PacketBuffer, Error> {
    let mut bytes = PacketBuffer::from(challenge.as_bytes().as_slice());
    noise.encrypt(sequence, &mut bytes)?;
    Ok(bytes)
}

fn decrypt(
    noise: &crypto::Transport,
    sequence: u64,
    mut bytes: PacketBuffer,
) -> Result<Digest, Error> {
    if bytes.len() != 48 {
        return Err(Error::Crypto);
    }
    noise.decrypt(sequence, &mut bytes)?;
    Ok(Digest::from_bytes(
        bytes.payload().try_into().map_err(|_| Error::Crypto)?,
    ))
}

impl SecureNode {
    /// Send one fixed challenge on an established diagnostic circuit owned by
    /// this source instance. The deadline is capped at ten seconds and the
    /// current circuit grant. At most 128 request correlations are retained;
    /// each observation admits at most one new request per second across CIDs.
    pub fn probe(
        &mut self,
        circuit: [u8; 32],
        measurement_id: EventId,
        challenge: Digest,
        deadline: Instant,
        carrier: &Carrier,
    ) -> Result<PendingFrame, Error> {
        let snapshot = self.identity.snapshot().clone();
        let reading = snapshot.reading().map_err(|_| Error::Authority)?;
        self.maintain_circuits(&reading, false)?;
        let mut owned = self.circuits.remove(&circuit).ok_or(Error::State)?;
        let result = self.probe_inner(
            &mut owned,
            measurement_id,
            challenge,
            deadline,
            carrier,
            &reading,
        );
        let result = owned.finish_operation(self.identity.snapshot(), "probe", result);
        self.circuits.insert(circuit, owned);
        result
    }

    fn probe_inner(
        &mut self,
        circuit: &mut Circuit,
        measurement_id: EventId,
        challenge: Digest,
        deadline: Instant,
        carrier: &Carrier,
        reading: &Reading<'_>,
    ) -> Result<PendingFrame, Error> {
        let snapshot = self.identity.snapshot();
        circuit.check_at(reading)?;
        let CircuitAuthorization::Diagnostic(observation) = circuit.context.authorization else {
            return Err(Error::Policy);
        };
        let State::Endpoint(noise) = &circuit.state else {
            return Err(Error::State);
        };
        let cfg = reading.configuration;
        if circuit.context.path.first() != Some(&cfg.node_id) {
            return Err(Error::Context);
        }
        authorization::source_instance(snapshot, cfg, circuit.context.authorization)?;
        if self.carriers.get(&circuit.context.path[1]) != Some(carrier) {
            return Err(Error::Context);
        }
        Self::check_outgoing_carrier(&self.carriers, &circuit.context, false, reading)?;
        let now = reading.instant;
        let now_ms = reading.now_ms;
        let mut guard = circuit.guard(snapshot, false);
        guard.expires_at = guard.expires_at.min(deadline).min(now + MAX_LIFETIME);
        let remaining = u64::try_from(guard.expires_at.saturating_duration_since(now).as_millis())
            .map_err(|_| Error::Limits)?;
        guard.expires_at_ms = guard
            .expires_at_ms
            .min(now_ms.checked_add(remaining).ok_or(Error::Limits)?);
        guard.finish()?;
        let metadata = ProbeMetadata {
            measurement_id,
            expires_at_ms: guard.expires_at_ms,
        };
        self.probes.admit(
            observation,
            circuit.context.circuit,
            measurement_id,
            RateRole::Source,
            reading,
        )?;
        guard.finish()?;
        let sequence = circuit.send[0].take()?;
        let frame = Frame {
            context: circuit.context.clone(),
            kind: Kind::ProbeRequest,
            reply: false,
            sequence,
            probe: Some(metadata),
            body: encrypt(noise, sequence, challenge)?,
            proofs: Vec::new(),
            exchange: None,
            auth: Default::default(),
        };
        let hop = self.handshake_hop(&frame, None, reading)?;
        let mut output = self.output(circuit, frame, hop, None, reading)?;
        output.guard = guard.clone();
        output.guard.finish()?;
        self.probes.pending.insert(
            (circuit.context.circuit, measurement_id),
            Association {
                metadata,
                challenge: Some(challenge),
                guard,
                completed: false,
            },
        );
        Ok(output)
    }

    pub(super) fn receive_probe(
        &mut self,
        circuit: &mut Circuit,
        frame: Frame,
        previous: NodeId,
        reading: &Reading<'_>,
    ) -> Result<Received, Error> {
        if !circuit.state.established() {
            return Err(Error::State);
        }
        let CircuitAuthorization::Diagnostic(observation) = circuit.context.authorization else {
            return Err(Error::Policy);
        };
        let metadata = frame.probe.ok_or(Error::Wire)?;
        let snapshot = self.identity.snapshot();
        circuit.check_at(reading)?;
        authorization::source_instance(
            snapshot,
            reading.configuration,
            circuit.context.authorization,
        )?;
        let now_ms = reading.now_ms;
        if now_ms >= metadata.expires_at_ms
            || metadata.expires_at_ms > circuit.expires_at_ms
            || metadata.expires_at_ms - now_ms > MAX_LIFETIME.as_millis() as u64
        {
            return Err(Error::Expired);
        }
        let hop = self.handshake_hop(&frame, Some(previous), reading)?;
        let index = usize::from(frame.reply);
        circuit.receive[index].check(frame.sequence)?;
        let sequence = frame.sequence;
        let key = (circuit.context.circuit, metadata.measurement_id);
        let result = if frame.reply {
            let request = self.probes.pending.get(&key).ok_or(Error::Replay)?;
            if request.completed || request.metadata != metadata {
                return Err(Error::Replay);
            }
            request.guard.ensure_current()?;
            let guard = request.guard.clone();
            let result = match (&circuit.state, hop) {
                (State::RelayEstablished, Hop::Forward(_)) => {
                    let mut output = self.output(circuit, frame, hop, None, reading)?;
                    output.guard = guard.clone();
                    Received::Forward(output)
                }
                (State::Endpoint(noise), Hop::Deliver) => {
                    let challenge = decrypt(noise, sequence, frame.body)?;
                    if request.challenge != Some(challenge) {
                        return Err(Error::Crypto);
                    }
                    Received::ProbeReply {
                        circuit: key.0,
                        measurement_id: key.1,
                        challenge,
                    }
                }
                _ => return Err(Error::State),
            };
            guard.finish()?;
            self.probes
                .pending
                .get_mut(&key)
                .ok_or(Error::State)?
                .completed = true;
            result
        } else {
            circuit.ensure_current(snapshot)?;
            self.probes
                .admit(observation, key.0, key.1, RateRole::Remote, reading)?;
            let mut guard = circuit.guard(snapshot, false);
            guard.expires_at_ms = metadata.expires_at_ms;
            guard.expires_at = guard
                .expires_at
                .min(reading.instant + Duration::from_millis(metadata.expires_at_ms - now_ms));
            let (result, challenge, completed) = match (&circuit.state, hop) {
                (State::RelayEstablished, Hop::Forward(_)) => {
                    let mut output = self.output(circuit, frame, hop, None, reading)?;
                    output.guard = guard.clone();
                    (Received::Forward(output), None, false)
                }
                (State::Endpoint(noise), Hop::Deliver) => {
                    let challenge = decrypt(noise, sequence, frame.body)?;
                    guard.finish()?;
                    let sequence = circuit.send[1].take()?;
                    let response = Frame {
                        context: circuit.context.clone(),
                        kind: Kind::ProbeReply,
                        reply: true,
                        sequence,
                        probe: Some(metadata),
                        body: encrypt(noise, sequence, challenge)?,
                        proofs: Vec::new(),
                        exchange: None,
                        auth: Default::default(),
                    };
                    let hop = self.handshake_hop(&response, None, reading)?;
                    let mut response = self.output(circuit, response, hop, None, reading)?;
                    response.guard = guard.clone();
                    (
                        Received::ProbeRequest {
                            circuit: key.0,
                            measurement_id: key.1,
                            response,
                        },
                        Some(challenge),
                        true,
                    )
                }
                _ => return Err(Error::State),
            };
            guard.finish()?;
            self.probes.pending.insert(
                key,
                Association {
                    metadata,
                    challenge,
                    guard,
                    completed,
                },
            );
            result
        };
        circuit.ensure_current(snapshot)?;
        circuit.receive[index].commit(sequence);
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
