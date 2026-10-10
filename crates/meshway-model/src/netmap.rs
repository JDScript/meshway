//! Per-node netmap references and leases. Content lives in `meshway-configuration`.
use crate::{Digest, Revision, Version};

/// Longest lease a Server may issue for one netmap.
pub const MAX_NETMAP_VALIDITY_MS: u64 = 24 * 60 * 60 * 1000;
/// Shortest configurable netmap validity.
pub const MIN_NETMAP_VALIDITY_MS: u64 = 5 * 60 * 1000;
/// Safety bound for one serialized netmap; exceeding it is a compiler defect.
pub const MAX_NETMAP_BYTES: usize = 8 * 1024 * 1024;

meshway_serde::object! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
/// Exact per-node netmap content: revision and serial identify it, the digest verifies it.
pub struct NetmapRef {
    /// Configuration revision whose compilation first produced this content.
    pub revision: Revision,
    /// Per-node content sequence; increases only when the content digest changes.
    pub serial: Version,
    /// Canonical content digest, excluding revision, serial, dependency digest and lease.
    pub digest: Digest,
}
}

meshway_serde::object! {
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Validity interval of one exact netmap; renewal replaces the lease, never the content.
pub struct NetmapLease {
    /// Exact netmap content covered by this lease.
    pub netmap: NetmapRef,
    /// Issue time in milliseconds since the Unix epoch, serialized as UTC RFC 3339.
    #[serde(rename = "issuedAt", with = "crate::timestamp")]
    pub issued_at_ms: u64 [value_type = crate::timestamp::Timestamp],
    /// Deadline in milliseconds since the Unix epoch, serialized as UTC RFC 3339; at most 24 hours after issue.
    #[serde(rename = "expiresAt", with = "crate::timestamp")]
    pub expires_at_ms: u64 [value_type = crate::timestamp::Timestamp],
}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("netmap lease interval is empty or longer than 24 hours")]
pub struct InvalidNetmapLease;

impl NetmapLease {
    pub fn new(
        netmap: NetmapRef,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<Self, InvalidNetmapLease> {
        let lease = Self {
            netmap,
            issued_at_ms,
            expires_at_ms,
        };
        lease.validate().map(|()| lease)
    }

    /// Received leases are checked again; the wire alone does not bound validity.
    pub fn validate(&self) -> Result<(), InvalidNetmapLease> {
        if self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms > MAX_NETMAP_VALIDITY_MS
        {
            return Err(InvalidNetmapLease);
        }
        Ok(())
    }

    /// Valid strictly before the deadline.
    pub fn valid_at(&self, now_ms: u64) -> bool {
        now_ms < self.expires_at_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference() -> NetmapRef {
        NetmapRef {
            revision: Revision::MIN,
            serial: Version::MIN,
            digest: Digest::from_bytes([7; 32]),
        }
    }

    #[test]
    fn lease_is_bounded_and_strict() {
        assert!(NetmapLease::new(reference(), 10, 10).is_err());
        assert!(NetmapLease::new(reference(), 10, 10 + MAX_NETMAP_VALIDITY_MS + 1).is_err());
        let lease = NetmapLease::new(reference(), 10, 10 + MAX_NETMAP_VALIDITY_MS).unwrap();
        assert!(lease.valid_at(9 + MAX_NETMAP_VALIDITY_MS));
        assert!(!lease.valid_at(10 + MAX_NETMAP_VALIDITY_MS));
    }

    #[test]
    fn lease_round_trips_and_rejects_unknown_fields() {
        let lease = NetmapLease::new(reference(), 1_000, 61_000).unwrap();
        let text = serde_json::to_string(&lease).unwrap();
        assert!(text.contains("\"issuedAt\":\"1970-01-01T00:00:01Z\""));
        assert_eq!(serde_json::from_str::<NetmapLease>(&text).unwrap(), lease);
        let extra = text.replacen('{', "{\"x\":1,", 1);
        assert!(serde_json::from_str::<NetmapLease>(&extra).is_err());
    }
}
