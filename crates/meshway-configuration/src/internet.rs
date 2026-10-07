//! The existing Meshway/Tailscale Internet address-set definition: IPv4 minus
//! RFC1918, CGNAT and link-local; IPv6 global unicast 2000::/3. Shared by declared
//! destination validation and execution consumers, not a DNS trust assertion.
use ipnet::IpNet;
use std::sync::OnceLock;

pub fn internet_ranges() -> &'static [IpNet] {
    static RANGES: OnceLock<Vec<IpNet>> = OnceLock::new();
    RANGES.get_or_init(|| {
        let excluded: Vec<IpNet> = [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "100.64.0.0/10",
            "169.254.0.0/16",
        ]
        .iter()
        .map(|s| s.parse().expect("constant prefix"))
        .collect();
        fn split(net: IpNet, excluded: &[IpNet], out: &mut Vec<IpNet>) {
            if excluded.iter().any(|e| e.contains(&net)) {
                return;
            }
            if excluded.iter().any(|e| net.contains(e)) {
                for child in net.subnets(net.prefix_len() + 1).expect("bounded split") {
                    split(child, excluded, out);
                }
            } else {
                out.push(net);
            }
        }
        let mut out = Vec::new();
        split(
            "0.0.0.0/0".parse().expect("constant prefix"),
            &excluded,
            &mut out,
        );
        out.push("2000::/3".parse().expect("constant prefix"));
        out
    })
}

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
