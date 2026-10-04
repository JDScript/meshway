//! Conservative handshake and in-order sequence association, NOT full TCP
//! conntrack/window validation. Supports ordinary SYN -> SYN/ACK -> ACK,
//! in-order data, the latest segment's retransmission, FIN and established RST.
//! No simultaneous-open, TCP Fast Open, out-of-order buffering, SACK recovery,
//! handshake RST, or sequence/window inference. Those packets fail closed.
use crate::{Direction, Error, TcpSegment};

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
    next: u32,
    last: TcpSegment,
    fin: bool,
}
impl Side {
    fn new(segment: TcpSegment) -> Self {
        Self {
            next: segment.end(),
            last: segment,
            fin: segment.fin(),
        }
    }
    fn accept(&mut self, segment: TcpSegment, peer_next: u32) -> Result<(), Error> {
        if segment == self.last {
            return Ok(());
        }
        // Requiring the latest peer sequence acknowledgement is deliberately
        // conservative; no receive-window scale or modular range is guessed.
        if !segment.ack()
            || segment.syn()
            || segment.sequence != self.next
            || segment.acknowledgement != peer_next
            || (self.fin && (segment.payload_len != 0 || segment.fin()))
        {
            return Err(Error::Flow);
        }
        self.next = segment.end();
        self.last = segment;
        self.fin |= segment.fin();
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
        if !self.established {
            match direction {
                Direction::Forward if segment == self.syn => return Ok(false),
                Direction::Reply => {
                    if self.syn_ack == Some(segment) {
                        return Ok(false);
                    }
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
                    self.forward.accept(segment, reply.next)?;
                    self.established = true;
                    return Ok(false);
                }
            }
        }
        let reply = self.reply.as_mut().ok_or(Error::Flow)?;
        // SYN retransmission is no longer a new handshake once established.
        if segment.syn() {
            return Err(Error::Flow);
        }
        match direction {
            Direction::Forward => self.forward.accept(segment, reply.next)?,
            Direction::Reply => reply.accept(segment, self.forward.next)?,
        }
        Ok(segment.rst())
    }
}
