//! Bounded TCP sequence association, not full TCP/window validation. Admitted
//! out-of-order ranges let endpoint TCP receive/SACK data after a path loss.
//! Only gap closure advances the cumulative ACK frontier. No payload reassembly,
//! simultaneous-open, TCP Fast Open, handshake RST or SACK/window inference.
use crate::{Direction, Error, TcpSegment};

/// Fixed constant-memory history bound, strictly below the serial half-space.
/// This is admitted sequence history, not an inferred TCP receive window.
const HISTORY: u32 = 16 * 1024 * 1024;
const MAX_AHEAD_RANGES: usize = 64;
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
    /// Disjoint, nonadjacent ranges strictly beyond next, in serial order.
    /// No buffer retention; empty in the normal contiguous fast path.
    ahead: Vec<std::ops::Range<u32>>,
}
impl Side {
    fn new(segment: TcpSegment) -> Self {
        Self {
            floor: segment.end(),
            next: segment.end(),
            fin: None,
            ahead: Vec::new(),
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
        let span = self.next.wrapping_sub(self.floor);
        let start = segment.sequence.wrapping_sub(self.floor);
        let consumed = u32::from(segment.payload_len) + u32::from(segment.fin());
        let limit = span + HISTORY;
        let end = start.checked_add(consumed).filter(|end| *end <= limit);
        if start > limit || end.is_none() {
            meshway_logging::profile::event(
                "tcp.reject",
                if segment.sequence.wrapping_sub(self.next) < (1 << 31) {
                    "future_window"
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
                    && payload_end.wrapping_sub(self.floor) > fin.wrapping_sub(self.floor))
                || start > fin.wrapping_sub(self.floor) + 1)
        {
            return Err(Error::Flow);
        }
        if segment.fin()
            && self.fin.is_none()
            && (payload_end.wrapping_sub(self.floor) < span
                || self.ahead.iter().any(|range| {
                    range.end.wrapping_sub(self.floor) > payload_end.wrapping_sub(self.floor)
                }))
        {
            // FIN cannot reinterpret an already admitted data byte or truncate
            // previously admitted ahead ranges. Its position is immutable.
            meshway_logging::profile::event("tcp.reject", "fin_reinterpretation");
            return Err(Error::Flow);
        }
        if consumed == 0 {
            // A pure ACK can carry SND.NXT after locally dropped data, but it
            // never records bytes, advances next or invents an ACK allowance.
            return Ok(());
        }
        let end = end.ok_or(Error::Flow)?;
        self.observe(start, end)?;
        if segment.fin() {
            self.fin = Some(payload_end);
        }
        Ok(())
    }

    /// Plan the bounded interval update before mutation. Sequence offsets span
    /// at most 2*HISTORY, far below the ambiguous half-space across u32 wrap.
    fn observe(&mut self, start: u32, end: u32) -> Result<(), Error> {
        let span = self.next.wrapping_sub(self.floor);
        if end <= span {
            return Ok(());
        }
        let upper = self
            .ahead
            .partition_point(|range| range.start.wrapping_sub(self.floor) <= end);
        let merged_end = if upper == 0 {
            end
        } else {
            end.max(self.ahead[upper - 1].end.wrapping_sub(self.floor))
        };
        if start <= span {
            if start < span {
                meshway_logging::profile::event("tcp.admit", "overlap_suffix");
            }
            self.ahead.drain(..upper);
            self.next = self.floor.wrapping_add(merged_end);
            if merged_end > HISTORY {
                self.floor = self.next.wrapping_sub(HISTORY);
            }
        } else {
            let lower = self
                .ahead
                .partition_point(|range| range.end.wrapping_sub(self.floor) < start);
            let kept = self.ahead.len() - (upper - lower);
            if kept == MAX_AHEAD_RANGES {
                meshway_logging::profile::event("tcp.reject", "range_capacity");
                return Err(Error::Capacity);
            }
            let merged_start = if lower == upper {
                start
            } else {
                start.min(self.ahead[lower].start.wrapping_sub(self.floor))
            };
            let range = self.floor.wrapping_add(merged_start)..self.floor.wrapping_add(merged_end);
            self.ahead.splice(lower..upper, std::iter::once(range));
            meshway_logging::profile::event("tcp.admit", "out_of_order");
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
    fn wrapped_gaps_merge_without_acknowledging_unobserved_bytes() {
        for isn in [100, u32::MAX - 4] {
            let mut state = established(isn, 900);
            let base = state.forward.next;
            state
                .accept(
                    Direction::Forward,
                    segment(base.wrapping_add(4), 901, 0x18, 4),
                )
                .unwrap();
            state
                .accept(
                    Direction::Forward,
                    segment(base.wrapping_add(9), 901, 0x18, 2),
                )
                .unwrap();
            assert_eq!(state.forward.next, base);
            assert_eq!(state.forward.ahead.len(), 2);
            state
                .accept(
                    Direction::Forward,
                    segment(base.wrapping_add(6), 901, 0x18, 4),
                )
                .unwrap();
            assert_eq!(state.forward.ahead.len(), 1);
            assert!(
                state
                    .accept(
                        Direction::Reply,
                        segment(901, base.wrapping_add(11), 0x10, 0)
                    )
                    .is_err()
            );
            state
                .accept(Direction::Reply, segment(901, base, 0x18, 5))
                .unwrap();
            state
                .accept(Direction::Forward, segment(base, 906, 0x18, 5))
                .unwrap();
            assert_eq!(state.forward.next, base.wrapping_add(11));
            assert!(state.forward.ahead.is_empty());
            state
                .accept(
                    Direction::Reply,
                    segment(906, base.wrapping_add(11), 0x10, 0),
                )
                .unwrap();
        }
    }

    #[test]
    fn gap_capacity_is_bounded_and_filling_holes_releases_slots_transactionally() {
        let mut state = established(u32::MAX - 63, 900);
        let base = state.forward.next;
        for i in 0..MAX_AHEAD_RANGES as u32 {
            state
                .accept(
                    Direction::Forward,
                    segment(base.wrapping_add(i * 2 + 1), 901, 0x18, 1),
                )
                .unwrap();
        }
        let original = state.forward.ahead.clone();
        assert_eq!(state.forward.next, base);
        assert!(matches!(
            state.accept(
                Direction::Forward,
                segment(base.wrapping_add(129), 901, 0x18, 1)
            ),
            Err(Error::Capacity)
        ));
        assert_eq!(state.forward.ahead, original);
        assert!(
            state
                .accept(
                    Direction::Forward,
                    segment(base.wrapping_add(HISTORY), 901, 0x18, 1)
                )
                .is_err()
        );
        assert_eq!(state.forward.ahead, original);
        // An empty ACK does not consume a range slot or advance the frontier.
        state
            .accept(
                Direction::Forward,
                segment(base.wrapping_add(1000), 901, 0x10, 0),
            )
            .unwrap();
        assert_eq!(state.forward.ahead, original);
        state
            .accept(Direction::Forward, segment(base, 901, 0x18, 1))
            .unwrap();
        assert_eq!(state.forward.ahead.len(), MAX_AHEAD_RANGES - 1);
        state
            .accept(
                Direction::Forward,
                segment(base.wrapping_add(129), 901, 0x18, 1),
            )
            .unwrap();
        state
            .accept(
                Direction::Forward,
                segment(base.wrapping_add(2), 901, 0x18, 127),
            )
            .unwrap();
        assert_eq!(state.forward.next, base.wrapping_add(130));
        assert!(state.forward.ahead.is_empty());
        state
            .accept(
                Direction::Reply,
                segment(901, base.wrapping_add(130), 0x10, 0),
            )
            .unwrap();
    }

    #[test]
    fn out_of_order_fin_keeps_one_boundary_and_cannot_reinterpret_admitted_data() {
        for reverse in [false, true] {
            let mut state = established(u32::MAX - 4, u32::MAX - 4);
            let base = state.forward.next;
            let direction = if reverse {
                Direction::Reply
            } else {
                Direction::Forward
            };
            let other = if reverse {
                Direction::Forward
            } else {
                Direction::Reply
            };
            state
                .accept(direction, segment(base.wrapping_add(4), base, 0x18, 4))
                .unwrap();
            let before = bounds(&state);
            assert!(
                state
                    .accept(direction, segment(base.wrapping_add(5), base, 0x11, 0))
                    .is_err()
            );
            assert_eq!(bounds(&state), before);
            state
                .accept(direction, segment(base.wrapping_add(4), base, 0x11, 4))
                .unwrap();
            assert!(
                state
                    .accept(direction, segment(base.wrapping_add(8), base, 0x18, 1))
                    .is_err()
            );
            assert!(
                state
                    .accept(direction, segment(base.wrapping_add(7), base, 0x11, 0))
                    .is_err()
            );
            assert!(
                state
                    .accept(other, segment(base, base.wrapping_add(9), 0x10, 0))
                    .is_err()
            );
            state
                .accept(direction, segment(base, base, 0x18, 4))
                .unwrap();
            state
                .accept(other, segment(base, base.wrapping_add(9), 0x10, 0))
                .unwrap();
            let completed = bounds(&state);
            state
                .accept(direction, segment(base.wrapping_add(6), base, 0x11, 2))
                .unwrap();
            assert_eq!(bounds(&state), completed);
        }
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
        for (seq, len, accepted, advanced) in [
            (floor.wrapping_sub(1), 1, false, 0),
            (next, 0, true, 0),
            (next.wrapping_add(1), 0, true, 0),
            (floor.wrapping_add(1 << 31), 0, false, 0),
            (next.wrapping_sub(1), 2, true, 1),
        ] {
            let mut candidate = state.clone();
            let result = candidate.accept(Direction::Forward, segment(seq, 901, 0x10, len));
            assert_eq!(result.is_ok(), accepted);
            assert_eq!(candidate.forward.next, next.wrapping_add(advanced));
            if !accepted {
                assert_eq!(bounds(&candidate), before);
            }
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
