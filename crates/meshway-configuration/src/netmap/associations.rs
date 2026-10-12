//! Trusted DNS context of one Node: names its exit resolver answered for a
//! source, as the Server accepted and signed them. Delivered beside the netmap
//! (never inside it): an association changes no netmap serial, has its own
//! deadline and is bound to the receiving Node's exact netmap.
use crate::{SerializationError, execution::ExecutionAddress, object::object};
use meshway_model::{Digest, NetmapRef, NodeId};
use std::collections::BTreeMap;

/// Associations of one source at most; the Server evicts the earliest expiry.
pub const MAX_SOURCE_ASSOCIATIONS: usize = 128;
/// Addresses of one association at most; a larger answer keeps the first ones.
pub const MAX_ASSOCIATION_ADDRESSES: usize = 16;
/// CNAME records of one chain at most.
pub const MAX_ASSOCIATION_CNAMES: usize = 8;
/// Associations of one overlay at most (own source and every served source);
/// the Server refuses a report that would put either Node's overlay above it.
pub const MAX_OVERLAY_ASSOCIATIONS: usize = 4096;
/// Shortest association life: a shorter TTL counts as this long.
pub const MIN_ASSOCIATION_LIFETIME_MS: u64 = 30_000;
/// Longest association life, whatever the TTL.
pub const MAX_ASSOCIATION_LIFETIME_MS: u64 = 3_600_000;

meshway_model::string_enum! {
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
/// Address record type of an answered query.
pub enum AssociationRecordType {
    /// IPv4 `A` records.
    A,
    /// IPv6 `AAAA` records.
    Aaaa,
}
}

impl AssociationRecordType {
    pub fn admits(self, address: std::net::IpAddr) -> bool {
        match self {
            Self::A => address.is_ipv4(),
            Self::Aaaa => address.is_ipv6(),
        }
    }
}

object! {
/// One name a source's exit resolver answered, for that source only.
pub struct DnsAssociation {
    /// Source Node whose query produced the answer.
    pub source: NodeId,
    /// Source egress selection identity the answer belongs to; another selection never reuses it.
    pub generation: Digest,
    /// Queried name: ASCII lowercase without a trailing dot.
    pub name: String,
    /// Record type of the answered query.
    pub record_type: AssociationRecordType,
    /// CNAME chain in answer order, from `name` to the owner of `addresses`; at most 8.
    pub cnames: Vec<String>,
    /// Public addresses of the answer for `recordType`, sorted and unique; 1 to 16.
    pub addresses: Vec<ExecutionAddress>,
    /// End of the association: the answer's shortest TTL (30 s to 1 h) after resolution.
    #[serde(rename = "expiresAt", with = "meshway_model::timestamp")]
    pub expires_at_ms: u64 [value_type = meshway_model::timestamp::Timestamp],
} }

object! {
/// Every association the receiving Node may use: those of its own queries and, at an exit resolver, those of the sources it serves.
pub struct DnsAssociationOverlay {
    /// Exact netmap of the receiving Node; any other netmap makes the overlay unusable.
    pub netmap: NetmapRef,
    /// Associations sorted by source, name and record type, at most one per triple.
    pub associations: Vec<DnsAssociation>,
} }

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("DNS association overlay violates its bounds")]
pub struct InvalidAssociations;

impl DnsAssociation {
    /// Whether this association names `address`.
    pub fn contains(&self, address: std::net::IpAddr) -> bool {
        self.addresses
            .iter()
            .any(|known| known.0.to_canonical() == address.to_canonical())
    }

    /// Bounds of one entry: the check every overlay applies to each entry,
    /// and the one the Server applies before it accepts a report.
    pub fn validate(&self) -> Result<(), InvalidAssociations> {
        let name = |value: &str| {
            !value.is_empty()
                && value.len() <= 253
                && !value.ends_with('.')
                && value == value.to_ascii_lowercase()
                && value.is_ascii()
        };
        let sorted = self.addresses.windows(2).all(|pair| pair[0].0 < pair[1].0);
        if !name(&self.name)
            || !self.cnames.iter().all(|cname| name(cname))
            || self.cnames.len() > MAX_ASSOCIATION_CNAMES
            || self.addresses.is_empty()
            || self.addresses.len() > MAX_ASSOCIATION_ADDRESSES
            || !sorted
            || !self
                .addresses
                .iter()
                .all(|address| self.record_type.admits(address.0))
        {
            return Err(InvalidAssociations);
        }
        Ok(())
    }
}

impl DnsAssociationOverlay {
    /// Canonical digest, the identity a Node names in its next sync.
    pub fn digest(&self) -> Result<Digest, SerializationError> {
        crate::digest(&("meshway-dns-associations", self))
    }

    /// Structural bounds: order, uniqueness, per-source and total counts,
    /// one selection identity per source and bounded entries.
    pub fn validate(&self) -> Result<(), InvalidAssociations> {
        if self.associations.len() > MAX_OVERLAY_ASSOCIATIONS {
            return Err(InvalidAssociations);
        }
        let mut sources: BTreeMap<NodeId, (Digest, usize)> = BTreeMap::new();
        for (index, association) in self.associations.iter().enumerate() {
            association.validate()?;
            if index > 0 {
                let previous = &self.associations[index - 1];
                if (previous.source, &previous.name, previous.record_type)
                    >= (
                        association.source,
                        &association.name,
                        association.record_type,
                    )
                {
                    return Err(InvalidAssociations);
                }
            }
            let entry = sources
                .entry(association.source)
                .or_insert((association.generation, 0));
            entry.1 += 1;
            if entry.0 != association.generation || entry.1 > MAX_SOURCE_ASSOCIATIONS {
                return Err(InvalidAssociations);
            }
        }
        Ok(())
    }
}
