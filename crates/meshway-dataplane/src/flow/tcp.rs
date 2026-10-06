//! Bounded contiguous TCP sequence association, NOT full conntrack/window
//! validation. Admission may precede a cancelled output: peer ACKs can lag the
//! admitted frontier, and retransmissions can split/coalesce admitted bytes.
//! No reassembly, new out-of-order data, simultaneous-open, TCP Fast Open,
//! handshake RST, window scaling or SACK inference.
use crate::{Direction, Error, TcpSegment};

/// Fixed constant-memory history bound, strictly below the serial half-space.
/// This is admitted sequence history, not an inferred TCP receive window.
const HISTORY: u32 = 16 * 1024 * 1024;
const _: () = assert!(HISTORY > 0 && HISTORY < (1 << 31));

#[derive(Clone)]
pub(super) struct TcpState {
    syn: TcpSegment,
    syn_ack: Option<TcpSegment>,
    established: bool,
    forward: Side,
    reply: Option<Side>,
}

#[derive(Clone)]
struct Side {
    floor: u32,
    next: u32,
    /// Sequence consumed by the original FIN; FIN is never reinterpreted as data.
    fin: Option<u32>,
}
impl Side {
    fn new(segment: TcpSegment) -> Self {
        Self {
            floor: segment.end(),
            next: segment.end(),
            fin: None,
        }
    }

    fn contains(&self, sequence: u32) -> bool {
        sequence.wrapping_sub(self.floor) <= self.next.wrapping_sub(self.floor)
    }

    fn accept(&mut self, segment: TcpSegment, peer: &Self) -> Result<(), Error> {
        // Never widen history based on an ACK, nor move its frontier backwards
        // for reordered ACKs. All comparisons are offsets within a <2^31 span.
        if !segment.ack() || segment.syn() || !peer.contains(segment.acknowledgement) {
            meshway_logging::profile::event("tcp.reject", "ack_or_flags");
            return Err(Error::Flow);
        }
        if segment.rst() {
            // Historical data is replayable; a reset is destructive and must
            // name the exact current sequence frontier with no data or FIN.
            return if segment.sequence == self.next && segment.payload_len == 0 && !segment.fin() {
                Ok(())
            } else {
                Err(Error::Flow)
            };
        }
        if !self.contains(segment.sequence) {
            meshway_logging::profile::event(
                "tcp.reject",
                if segment.sequence.wrapping_sub(self.next) < (1 << 31) {
                    "future_sequence"
                } else {
                    "old_sequence"
                },
            );
            return Err(Error::Flow);
        }
        let payload_end = segment
            .sequence
            .wrapping_add(u32::from(segment.payload_len));
        if let Some(fin) = self.fin
            && ((segment.fin() && payload_end != fin)
                || (!segment.fin()
                    && segment.payload_len != 0
                    && payload_end.wrapping_sub(self.floor) > fin.wrapping_sub(self.floor)))
        {
            return Err(Error::Flow);
        }
        let consumed = u32::from(segment.payload_len) + u32::from(segment.fin());
        if segment.sequence != self.next || self.fin.is_some() {
            // Retransmit only a wholly admitted range. Partial overlap followed
            // by new bytes is unsupported, as is inventing a FIN in old data.
            if consumed > self.next.wrapping_sub(segment.sequence)
                || (segment.fin() && self.fin.is_none())
            {
                meshway_logging::profile::event("tcp.reject", "partial_overlap_or_fin");
                return Err(Error::Flow);
            }
            return Ok(());
        }
        // Only new contiguous sequence consumption advances the bounded floor.
        // Every fallible check precedes mutation; rejected segments are inert.
        if segment.fin() {
            self.fin = Some(payload_end);
        }
        let span = self.next.wrapping_sub(self.floor) + consumed;
        self.next = segment.end();
        if span > HISTORY {
            self.floor = self.next.wrapping_sub(HISTORY);
        }
        Ok(())
    }
}

impl TcpState {
    pub fn new(syn: TcpSegment) -> Result<Self, Error> {
        if !syn.syn()
            || syn.ack()
            || syn.rst()
            || syn.fin()
            || syn.payload_len != 0
            || syn.acknowledgement != 0
            || syn.flags & !(0x02 | 0x40 | 0x80) != 0
        {
            return Err(Error::Flow);
        }
        Ok(Self {
            syn,
            syn_ack: None,
            established: false,
            forward: Side::new(syn),
            reply: None,
        })
    }

    /// Returns true when the entry should be removed after accepting a reset.
    pub fn accept(&mut self, direction: Direction, segment: TcpSegment) -> Result<bool, Error> {
        // These exact handshake facts remain valid retransmissions even after
        // establishment (the earlier admitted output may never have arrived).
        // They do not reset sequence/FIN state or bypass validation for new SYNs.
        if (direction == Direction::Forward && segment == self.syn)
            || (direction == Direction::Reply && self.syn_ack == Some(segment))
        {
            return Ok(false);
        }
        if !self.established {
            match direction {
                Direction::Reply => {
                    if self.syn_ack.is_some()
                        || !segment.syn()
                        || !segment.ack()
                        || segment.flags & !(0x02 | 0x10 | 0x40) != 0
                        || segment.acknowledgement != self.forward.next
                        || segment.payload_len != 0
                    {
                        return Err(Error::Flow);
                    }
                    self.syn_ack = Some(segment);
                    self.reply = Some(Side::new(segment));
                    return Ok(false);
                }
                Direction::Forward => {
                    if segment.rst() {
                        return Err(Error::Flow);
                    }
                    let reply = self.reply.as_ref().ok_or(Error::Flow)?;
                    self.forward.accept(segment, reply)?;
                    self.established = true;
                    return Ok(false);
                }
            }
        }
        let reply = self.reply.as_mut().ok_or(Error::Flow)?;
        // A different SYN never restarts the established tuple.
        if segment.syn() {
            return Err(Error::Flow);
        }
        match direction {
            Direction::Forward => self.forward.accept(segment, reply)?,
            Direction::Reply => reply.accept(segment, &self.forward)?,
        }
        Ok(segment.rst())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(sequence: u32, acknowledgement: u32, flags: u8, payload_len: u16) -> TcpSegment {
        TcpSegment {
            sequence,
            acknowledgement,
            flags,
            payload_len,
        }
    }

    fn established(source: u32, terminal: u32) -> TcpState {
        let mut state = TcpState::new(segment(source, 0, 2, 0)).unwrap();
        state
            .accept(
                Direction::Reply,
                segment(terminal, source.wrapping_add(1), 0x12, 0),
            )
            .unwrap();
        state
            .accept(
                Direction::Forward,
                segment(source.wrapping_add(1), terminal.wrapping_add(1), 0x10, 0),
            )
            .unwrap();
        state
    }

    fn bounds(state: &TcpState) -> [(u32, u32, Option<u32>); 2] {
        let reply = state.reply.as_ref().unwrap();
        [
            (state.forward.floor, state.forward.next, state.forward.fin),
            (reply.floor, reply.next, reply.fin),
        ]
    }

    #[test]
    fn retained_serial_floor_is_bounded_and_rejected_ack_cannot_expand_or_shrink_it() {
        let mut state = established(u32::MAX - 100, 900);
        // Metadata-only constant-memory state exercise; no packet buffers,
        // sockets, timers or performance thresholds are involved.
        for _ in 0..=HISTORY / 60_000 {
            state
                .accept(
                    Direction::Forward,
                    segment(state.forward.next, 901, 0x18, 60_000),
                )
                .unwrap();
        }
        assert_eq!(
            state.forward.next.wrapping_sub(state.forward.floor),
            HISTORY
        );
        let before = bounds(&state);
        let floor = state.forward.floor;
        let next = state.forward.next;
        for ack in [
            next.wrapping_add(1),
            floor.wrapping_sub(1),
            floor.wrapping_add(1 << 31),
        ] {
            assert!(matches!(
                state.accept(Direction::Reply, segment(901, ack, 0x10, 0)),
                Err(Error::Flow)
            ));
            assert_eq!(bounds(&state), before);
        }
        for ack in [next, floor, next.wrapping_sub(1), floor] {
            state
                .accept(Direction::Reply, segment(901, ack, 0x10, 0))
                .unwrap();
            assert_eq!(bounds(&state), before);
        }
        state
            .accept(Direction::Forward, segment(floor, 901, 0x18, 60_000))
            .unwrap();
        for (seq, len) in [
            (floor.wrapping_sub(1), 1),
            (next, 0),
            (next.wrapping_add(1), 0),
            (floor.wrapping_add(1 << 31), 0),
            (next.wrapping_sub(1), 2),
        ] {
            let result = state.accept(Direction::Forward, segment(seq, 901, 0x10, len));
            assert_eq!(result.is_ok(), seq == next && len == 0);
            assert_eq!(bounds(&state), before);
        }
        state
            .accept(Direction::Forward, segment(next, 901, 0x18, 3))
            .unwrap();
        assert_eq!(state.forward.floor, floor.wrapping_add(3));
        assert_eq!(state.forward.next, next.wrapping_add(3));
        let advanced = bounds(&state);
        // Saved handshake context is separate from the sliding data/ACK history.
        state.accept(Direction::Forward, state.syn).unwrap();
        state
            .accept(Direction::Reply, state.syn_ack.unwrap())
            .unwrap();
        assert!(state.established);
        assert_eq!(bounds(&state), advanced);
    }

    #[test]
    fn wrapped_ack_and_retransmission_intervals_use_serial_offsets_in_both_directions() {
        let mut state = established(u32::MAX - 8, u32::MAX - 16);
        let a = state.forward.next;
        let b = state.reply.as_ref().unwrap().next;
        state
            .accept(Direction::Forward, segment(a, b, 0x18, 12))
            .unwrap();
        state
            .accept(Direction::Reply, segment(b, a.wrapping_add(12), 0x18, 20))
            .unwrap();
        let before = bounds(&state);
        state
            .accept(Direction::Forward, segment(a.wrapping_add(3), b, 0x18, 8))
            .unwrap();
        state
            .accept(Direction::Reply, segment(b.wrapping_add(7), a, 0x18, 10))
            .unwrap();
        for (direction, seq, ack) in [
            (Direction::Forward, a.wrapping_add(12), b.wrapping_sub(1)),
            (Direction::Reply, b.wrapping_add(20), a.wrapping_add(13)),
        ] {
            assert!(matches!(
                state.accept(direction, segment(seq, ack, 0x10, 0)),
                Err(Error::Flow)
            ));
            assert_eq!(bounds(&state), before);
        }
        state
            .accept(
                Direction::Forward,
                segment(a.wrapping_add(12), b.wrapping_add(20), 0x11, 2),
            )
            .unwrap();
        let closed = bounds(&state);
        state
            .accept(Direction::Forward, segment(a.wrapping_add(13), b, 0x11, 1))
            .unwrap();
        assert_eq!(bounds(&state), closed);
        assert!(matches!(
            state.accept(Direction::Forward, segment(a.wrapping_add(15), b, 0x18, 1)),
            Err(Error::Flow)
        ));
    }
}
