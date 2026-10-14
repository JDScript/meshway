//! Closed address intervals: the one merge and minimal CIDR cover shared by
//! rule sets, the Policy egress partition and the Underlay capture table.
use std::net::{Ipv4Addr, Ipv6Addr};

/// An address family's integer form.
pub trait Address: Copy + Ord {
    /// Address width in bits.
    const BITS: u32;
    fn next(self) -> Option<Self>;
    fn previous(self) -> Option<Self>;
    /// Last address of the block of `bits` prefix length starting at `self`.
    fn block_end(self, bits: u32) -> Option<Self>;
    fn trailing_zeros(self) -> u32;
}

macro_rules! address {
    ($t:ty) => {
        impl Address for $t {
            const BITS: u32 = <$t>::BITS;
            fn next(self) -> Option<Self> {
                self.checked_add(1)
            }
            fn previous(self) -> Option<Self> {
                self.checked_sub(1)
            }
            fn block_end(self, bits: u32) -> Option<Self> {
                self.checked_add(<$t>::MAX.checked_shr(bits).unwrap_or(0))
            }
            fn trailing_zeros(self) -> u32 {
                <$t>::trailing_zeros(self)
            }
        }
    };
}
address!(u32);
address!(u128);

macro_rules! ip_address {
    ($ip:ty, $int:ty) => {
        impl Address for $ip {
            const BITS: u32 = <$int>::BITS;
            fn next(self) -> Option<Self> {
                <$int>::from(self).next().map(Self::from)
            }
            fn previous(self) -> Option<Self> {
                <$int>::from(self).previous().map(Self::from)
            }
            fn block_end(self, bits: u32) -> Option<Self> {
                <$int>::from(self).block_end(bits).map(Self::from)
            }
            fn trailing_zeros(self) -> u32 {
                <$int>::from(self).trailing_zeros()
            }
        }
    };
}
ip_address!(Ipv4Addr, u32);
ip_address!(Ipv6Addr, u128);

/// Sorted, merged ranges: overlapping and touching ranges become one.
pub fn merge<T: Address>(mut ranges: Vec<(T, T)>) -> Vec<(T, T)> {
    ranges.sort_unstable();
    let mut out: Vec<(T, T)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        match out.last_mut() {
            Some(last) if last.1.next().is_none_or(|after| start <= after) => {
                last.1 = last.1.max(end);
            }
            _ => out.push((start, end)),
        }
    }
    out
}

/// Whether `ranges` is what [`merge`] returns: ordered, disjoint and
/// non-adjacent.
pub fn is_merged<T: Address>(ranges: &[(T, T)]) -> bool {
    ranges.iter().all(|(start, end)| start <= end)
        && ranges
            .windows(2)
            .all(|pair| pair[0].1.next().is_some_and(|after| after < pair[1].0))
}

/// The minimal CIDR cover of `[start, end]` in ascending order, as
/// (network, prefix length); computed lazily, so counting allocates nothing.
pub fn cover<T: Address>(start: T, end: T) -> Cover<T> {
    Cover {
        at: (start <= end).then_some(start),
        end,
    }
}

/// Iterator of [`cover`].
#[derive(Clone, Debug)]
pub struct Cover<T> {
    at: Option<T>,
    end: T,
}

impl<T: Address> Iterator for Cover<T> {
    type Item = (T, u8);
    fn next(&mut self) -> Option<(T, u8)> {
        let at = self.at?;
        let mut bits = T::BITS - at.trailing_zeros().min(T::BITS);
        while at.block_end(bits).is_none_or(|last| last > self.end) {
            bits += 1;
        }
        let last = at.block_end(bits)?;
        self.at = last.next().filter(|next| *next <= self.end);
        Some((at, bits as u8))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_joins_overlapping_and_touching() {
        assert_eq!(merge(vec![(5u32, 9), (0, 4), (11, 12)]), [(0, 9), (11, 12)]);
        assert_eq!(merge(vec![(0u128, u128::MAX), (3, 4)]), [(0, u128::MAX)]);
        assert_eq!(merge(vec![(3u32, 3), (1, 8)]), [(1, 8)]);
        assert!(is_merged(&[(0u32, 9), (11, 12)]));
        assert!(!is_merged(&[(0u32, 9), (10, 12)]));
        assert!(!is_merged(&[(0u32, u32::MAX), (0, 1)]));
    }

    #[test]
    fn cover_is_minimal_and_ordered() {
        assert_eq!(cover(0u32, u32::MAX).collect::<Vec<_>>(), [(0, 0)]);
        assert_eq!(
            cover(1u32, 6).collect::<Vec<_>>(),
            [(1, 32), (2, 31), (4, 31), (6, 32)]
        );
        assert_eq!(cover(1u32, u32::MAX).count(), 32);
        assert_eq!(cover(10u32, 10).count(), 1);
        assert_eq!(cover(0u32, 2).count(), 2);
        assert_eq!(cover(0u128, u128::MAX).count(), 1);
        assert_eq!(
            cover(u128::MAX, u128::MAX).collect::<Vec<_>>(),
            [(u128::MAX, 128)]
        );
        assert_eq!(cover(5u32, 4).count(), 0);
    }
}
