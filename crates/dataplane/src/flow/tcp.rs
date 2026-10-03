//! TCP progress is updated on successful local enqueue/device delivery. Proof of
//! reverse eligibility is a separate flow state; TCP data is never stop-and-wait
//! on a Meshway receipt. Reordering is bounded, not treated as a new connection.
use super::packet::Tcp;

#[derive(Clone, Copy)]
pub(super) struct State {
    syn: u32,
    peer_syn: Option<u32>,
    end: [u32; 2],
    established: bool,
    fin: [Option<u32>; 2],
    fin_acked: [bool; 2],
    reset: bool,
    scale: [Option<u8>; 2],
    window: [u32; 2],
}

impl State {
    pub fn new(packet: Tcp) -> Self {
        let syn = packet.seq;
        Self {
            syn,
            peer_syn: None,
            end: [syn.wrapping_add(1), 0],
            established: false,
            fin: [None, None],
            fin_acked: [false, false],
            reset: false,
            scale: [packet.scale, None],
            window: [u32::from(packet.window), 65535],
        }
    }

    pub fn terminal(&self) -> bool {
        self.reset || self.fin_acked == [true, true]
    }

    pub fn advance(&mut self, tcp: Tcp, reply: bool, sending: bool) -> bool {
        if self.reset {
            return false;
        }
        if tcp.flags == 2 {
            return !reply
                && !self.established
                && tcp.seq == self.syn
                && tcp.len == 0
                && tcp.scale == self.scale[0];
        }
        if tcp.flags == 0x12 {
            if !reply
                || tcp.ack != self.syn.wrapping_add(1)
                || tcp.len != 0
                || self.peer_syn.is_some_and(|seq| seq != tcp.seq)
                || (self.peer_syn.is_some() && self.scale[1] != tcp.scale)
            {
                return false;
            }
            if self.peer_syn.is_none() {
                self.peer_syn = Some(tcp.seq);
                self.end[1] = tcp.seq.wrapping_add(1);
                self.scale[1] = tcp.scale;
                self.window[1] = u32::from(tcp.window);
            }
            // An exact SYNACK retransmit remains valid after our ACK was lost.
            return true;
        }
        if reply
            && tcp.flags == 0x14
            && self.peer_syn.is_none()
            && tcp.ack == self.syn.wrapping_add(1)
            && tcp.seq == 0
            && tcp.len == 0
        {
            self.reset = true;
            return true;
        }
        if tcp.flags & 0x10 == 0 || tcp.flags & 0x22 != 0 || self.peer_syn.is_none() {
            return false;
        }
        let direction = usize::from(reply);
        let other = 1 - direction;
        let scale = if self.scale.iter().all(Option::is_some) {
            self.scale[direction].unwrap_or(0)
        } else {
            0
        };
        self.window[direction] = self.window[direction].max(u32::from(tcp.window) << scale);
        // Observed peer receive windows bound flight/reordering. One maximum IP
        // segment of slack covers an in-flight final segment/FIN and stale ACK;
        // an arbitrary short wrapper window must not block valid TCP retries.
        let sequence_window = self.window[other].max(65535) + 65535;
        let ack_window = self.window[direction].max(65535) + 65535;
        let offset = tcp.seq.wrapping_sub(self.end[direction]) as i32;
        if offset.unsigned_abs() > sequence_window
            || (sending && offset > 0)
            || self.end[other].wrapping_sub(tcp.ack) > ack_window
        {
            return false;
        }
        if !self.established {
            if reply || tcp.seq != self.end[0] || tcp.ack != self.end[1] {
                return false;
            }
            self.established = true;
        }
        let end = tcp
            .seq
            .wrapping_add(tcp.len + u32::from(tcp.flags & 1 != 0));
        if self.fin[direction].is_some_and(|fin| (end.wrapping_sub(fin) as i32) > 0) {
            return false;
        }
        if tcp.flags & 1 != 0 {
            if self.fin[direction].is_some_and(|fin| fin != end) {
                return false;
            }
            self.fin[direction] = Some(end);
        }
        if (end.wrapping_sub(self.end[direction]) as i32) > 0 {
            self.end[direction] = end;
        }
        if let Some(fin) = self.fin[other]
            && (tcp.ack.wrapping_sub(fin) as i32) >= 0
        {
            self.fin_acked[other] = true;
        }
        self.reset |= tcp.flags & 4 != 0;
        true
    }
}
