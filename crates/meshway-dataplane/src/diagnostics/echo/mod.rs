//! ICMP echo toward an address outside the Mesh, with one request in flight.
//! This owner builds requests, classifies answers and times them; it has no
//! socket and no authority. A request through the Mesh is an ordinary local
//! packet: SecureNode decides it like any first packet, carries it on the
//! exit path and returns the reply or ICMP error on the original full path.
use crate::{Packet, PacketFacts, Transport};
use std::{
    net::{IpAddr, Ipv4Addr},
    time::{Duration, Instant},
};

/// Bytes of echo payload after the 8-byte ICMP header.
const PAYLOAD: usize = 32;

/// What answered one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Reply,
    /// ICMP destination unreachable or time exceeded quoting the request.
    Unreachable {
        kind: u8,
        code: u8,
    },
    Timeout,
    /// Never admitted by this Node.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Result {
    pub sequence: u16,
    pub outcome: Outcome,
    /// Source of the answer.
    pub from: Option<IpAddr>,
    /// Admission to receipt, on the monotonic clock.
    pub rtt: Option<Duration>,
    /// The request waited for a circuit to be set up; `rtt` includes it.
    pub setup: bool,
}

struct InFlight {
    sequence: u16,
    admitted: Option<Instant>,
    setup: bool,
    deadline: Instant,
}

pub struct Echo {
    destination: Ipv4Addr,
    identifier: u16,
    sequence: u16,
    in_flight: Option<InFlight>,
    next: Instant,
}

impl Echo {
    pub fn new(destination: Ipv4Addr, identifier: u16, now: Instant) -> Self {
        Self {
            destination,
            identifier,
            sequence: 0,
            in_flight: None,
            next: now,
        }
    }

    pub fn identifier(&self) -> u16 {
        self.identifier
    }

    pub fn destination(&self) -> Ipv4Addr {
        self.destination
    }

    /// A new request may be sent now: none is in flight and the interval passed.
    pub fn due(&self, now: Instant) -> bool {
        self.in_flight.is_none() && now >= self.next
    }

    /// The next event to wake for: the in-flight deadline or the next send.
    pub fn wake_at(&self) -> Instant {
        self.in_flight
            .as_ref()
            .map_or(self.next, |flight| flight.deadline)
    }

    /// The ICMP message of the next request (header and payload), and its
    /// sequence. Its deadline runs from now; admission is recorded separately.
    pub fn message(&mut self, now: Instant) -> (u16, Vec<u8>) {
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1);
        self.in_flight = Some(InFlight {
            sequence,
            admitted: None,
            setup: false,
            deadline: now + super::DEADLINE,
        });
        self.next = now + super::MIN_INTERVAL;
        let mut message = vec![0; 8 + PAYLOAD];
        message[0] = 8;
        message[4..6].copy_from_slice(&self.identifier.to_be_bytes());
        message[6..8].copy_from_slice(&sequence.to_be_bytes());
        for (index, byte) in message[8..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        let checksum = checksum(&message);
        message[2..4].copy_from_slice(&checksum.to_be_bytes());
        (sequence, message)
    }

    /// The next request as a complete IPv4 packet from `source`, this Node's
    /// Mesh address, for SecureNode's local admission.
    pub fn packet(
        &mut self,
        source: Ipv4Addr,
        now: Instant,
    ) -> std::result::Result<Packet, crate::PacketError> {
        let (sequence, message) = self.message(now);
        let length = 20 + message.len();
        let mut bytes = Vec::with_capacity(length);
        bytes.extend_from_slice(&[0x45, 0]);
        bytes.extend_from_slice(&(length as u16).to_be_bytes());
        bytes.extend_from_slice(&sequence.to_be_bytes());
        bytes.extend_from_slice(&[0x40, 0, 64, 1, 0, 0]);
        bytes.extend_from_slice(&source.octets());
        bytes.extend_from_slice(&self.destination.octets());
        let header = checksum(&bytes);
        bytes[10..12].copy_from_slice(&header.to_be_bytes());
        bytes.extend_from_slice(&message);
        Packet::parse(bytes)
    }

    /// Whether `facts` is this owner's in-flight request.
    pub fn is_request(&self, facts: &PacketFacts) -> bool {
        matches!(facts.transport, Transport::Echo { reply: false, identifier, sequence }
            if identifier == self.identifier
                && self.in_flight.as_ref().is_some_and(|flight| flight.sequence == sequence))
            && facts.traffic.destination == IpAddr::V4(self.destination)
    }

    /// This Node admitted the in-flight request at `at`; `setup` when it
    /// waits for a circuit first. Later admissions do not move the clock.
    pub fn admitted(&mut self, at: Instant, setup: bool) {
        if let Some(flight) = &mut self.in_flight
            && flight.admitted.is_none()
        {
            flight.admitted = Some(at);
            flight.setup = setup;
        }
    }

    /// This Node refused the in-flight request.
    pub fn refused(&mut self, now: Instant) -> Option<Result> {
        let flight = self.in_flight.take()?;
        self.next = self.next.max(now);
        Some(Result {
            sequence: flight.sequence,
            outcome: Outcome::Failed,
            from: None,
            rtt: None,
            setup: false,
        })
    }

    /// Classify an answer: the echo reply from the destination, or an ICMP
    /// error quoting the in-flight request. Anything else is not ours.
    pub fn answer(&mut self, facts: &PacketFacts, at: Instant) -> Option<Result> {
        let flight = self.in_flight.as_ref()?;
        let outcome = match facts.transport {
            Transport::Echo {
                reply: true,
                identifier,
                sequence,
            } if identifier == self.identifier
                && sequence == flight.sequence
                && facts.traffic.source == IpAddr::V4(self.destination) =>
            {
                Outcome::Reply
            }
            Transport::Error(error)
                if error.quoted.echo == Some((self.identifier, flight.sequence))
                    && error.quoted.traffic.destination == IpAddr::V4(self.destination) =>
            {
                Outcome::Unreachable {
                    kind: error.kind,
                    code: error.code,
                }
            }
            _ => return None,
        };
        let admitted = flight.admitted?;
        if at > flight.deadline {
            return None;
        }
        let flight = self.in_flight.take()?;
        Some(Result {
            sequence: flight.sequence,
            outcome,
            from: Some(facts.traffic.source),
            rtt: Some(at.saturating_duration_since(admitted)),
            setup: flight.setup,
        })
    }

    /// The in-flight request's deadline passed: a timeout once admitted.
    pub fn expire(&mut self, now: Instant) -> Option<Result> {
        if self.in_flight.as_ref()?.deadline > now {
            return None;
        }
        let flight = self.in_flight.take()?;
        Some(Result {
            sequence: flight.sequence,
            outcome: if flight.admitted.is_some() {
                Outcome::Timeout
            } else {
                Outcome::Failed
            },
            from: None,
            rtt: None,
            setup: flight.setup,
        })
    }
}

fn checksum(bytes: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in bytes.chunks(2) {
        let word = u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests;
