use super::Error;

/// A sliding, 1024-packet replay window. Validation is read-only; only a fully
/// authenticated and admitted packet commits its sequence.
#[derive(Default)]
pub(super) struct ReplayWindow {
    highest: Option<u64>,
    bits: [u64; 16],
}

impl ReplayWindow {
    pub fn check(&self, nonce: u64) -> Result<(), Error> {
        if nonce == u64::MAX {
            return Err(Error::Nonce);
        }
        if let Some(highest) = self.highest
            && nonce <= highest
        {
            let distance = highest - nonce;
            if distance >= 1024 || self.bits[distance as usize / 64] & (1 << (distance % 64)) != 0 {
                return Err(Error::Replay);
            }
        }
        Ok(())
    }

    pub fn commit(&mut self, nonce: u64) {
        match self.highest {
            None => {
                self.highest = Some(nonce);
                self.bits[0] = 1;
            }
            Some(highest) if nonce > highest => {
                let shift = nonce - highest;
                if shift >= 1024 {
                    self.bits = [0; 16];
                } else {
                    let words = shift as usize / 64;
                    let bits = shift as usize % 64;
                    for i in (0..16).rev() {
                        self.bits[i] = if i >= words {
                            self.bits[i - words] << bits
                        } else {
                            0
                        };
                        if bits != 0 && i > words {
                            self.bits[i] |= self.bits[i - words - 1] >> (64 - bits);
                        }
                    }
                }
                self.highest = Some(nonce);
                self.bits[0] |= 1;
            }
            Some(highest) => {
                let distance = (highest - nonce) as usize;
                self.bits[distance / 64] |= 1 << (distance % 64);
            }
        }
    }
}

#[derive(Default)]
pub(super) struct SendNonce(u64);
impl SendNonce {
    pub fn take(&mut self) -> Result<u64, Error> {
        let next = self.0.checked_add(1).ok_or(Error::Nonce)?;
        let current = self.0;
        self.0 = next;
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_out_of_order_and_window_edges() {
        let mut window = ReplayWindow::default();
        window.check(0).unwrap();
        window.commit(0);
        assert!(window.check(0).is_err());
        window.commit(1000);
        window.check(999).unwrap();
        window.commit(999);
        assert!(window.check(999).is_err());
        assert!(window.check(0).is_err());
        window.commit(1024);
        assert!(window.check(0).is_err());
        window.check(1).unwrap();
        window.commit(1);
        assert!(window.check(1).is_err());
        window.commit(65_535);
        window.check(65_534).unwrap();
        assert!(window.check(1024).is_err());
        assert!(window.check(u64::MAX).is_err());
    }

    #[test]
    fn uncommitted_checks_and_directions_do_not_consume() {
        let mut forward = ReplayWindow::default();
        let reply = ReplayWindow::default();
        forward.check(7).unwrap();
        forward.check(7).unwrap();
        forward.commit(7);
        assert!(forward.check(7).is_err());
        reply.check(7).unwrap();
    }

    #[test]
    fn send_nonce_never_wraps() {
        let mut nonce = SendNonce(u64::MAX - 1);
        assert_eq!(nonce.take().unwrap(), u64::MAX - 1);
        assert!(nonce.take().is_err());
        assert!(nonce.take().is_err());
    }
}
