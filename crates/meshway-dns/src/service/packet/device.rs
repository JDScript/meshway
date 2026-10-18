//! Bounded L3 queue for the DNS-only TCP/UDP endpoint. Never forwards packets.
use smoltcp::{
    phy::{self, Device, DeviceCapabilities, Medium},
    time::Instant,
};
use std::collections::VecDeque;

pub(super) const BATCH: usize = 32;
pub(super) struct Queue {
    pub input: VecDeque<Vec<u8>>,
    pub output: VecDeque<Vec<u8>>,
    pub mtu: usize,
}
impl Device for Queue {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;
    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ip;
        capabilities.max_transmission_unit = self.mtu;
        capabilities.max_burst_size = Some(BATCH);
        capabilities
    }
    fn receive(&mut self, _: Instant) -> Option<(Rx, Tx<'_>)> {
        if self.output.len() >= BATCH {
            return None;
        }
        self.input
            .pop_front()
            .map(|bytes| (Rx(bytes), Tx(&mut self.output)))
    }
    fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
        (self.output.len() < BATCH).then_some(Tx(&mut self.output))
    }
}
pub(super) struct Rx(Vec<u8>);
impl phy::RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
pub(super) struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);
impl phy::TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut bytes = vec![0; len];
        let result = f(&mut bytes);
        self.0.push_back(bytes);
        result
    }
}
