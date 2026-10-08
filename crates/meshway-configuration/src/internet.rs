//! Generic CIDR set operations; address classification belongs to Policy.
use ipnet::IpNet;

/// An aggregated set of address ranges. Building it merges adjacent and
/// nested grants once; each coverage query is then a binary search, so
/// checking thousands of destinations against thousands of grants stays
/// linear instead of re-aggregating per destination.
#[derive(Clone, Debug, Default)]
pub struct AddressSet {
    /// Sorted, pairwise disjoint, IPv4 before IPv6.
    ranges: Vec<IpNet>,
}

impl AddressSet {
    pub fn new(grants: impl IntoIterator<Item = IpNet>) -> Self {
        let mut ranges = IpNet::aggregate(&grants.into_iter().collect::<Vec<_>>());
        ranges.sort();
        Self { ranges }
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Whether one aggregated range contains the whole target.
    pub fn covers(&self, target: &IpNet) -> bool {
        // Ranges are disjoint and sorted by network address; the only range
        // that can contain `target` is the last one that starts at or before it.
        let index = self.ranges.partition_point(|range| {
            matches!((range, target), (IpNet::V4(_), IpNet::V6(_)))
                || (std::mem::discriminant(range) == std::mem::discriminant(target)
                    && range.network() <= target.network())
        });
        index
            .checked_sub(1)
            .is_some_and(|i| self.ranges[i].contains(target))
    }
}
