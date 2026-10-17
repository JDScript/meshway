//! Default-zone conntrack identity and exact deletion over ctnetlink. Never
//! flushes conntrack. The only conntrack owner; the exit backend calls it.
use super::{ExitError, tuple::OriginalTuple};
use meshway_model::exit::ExitUnavailableReason as Reason;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

mod netlink;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tuple {
    pub source: IpAddr,
    pub destination: IpAddr,
    pub protocol: u8,
    pub source_key: u16,
    pub destination_key: u16,
}

impl Tuple {
    pub fn key(&self) -> serde_json::Value {
        serde_json::json!({"concat": [self.source.to_string(), self.destination.to_string(), self.protocol, self.source_key, self.destination_key]})
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetiredTuple {
    pub tuple: Tuple,
    pub sequence: u16,
    /// UTC retirement time; 0 for a record written before it was kept.
    #[serde(default)]
    pub retired_ms: u64,
}
impl RetiredTuple {
    fn echo(&self) -> bool {
        matches!(self.tuple.protocol, 1 | 58)
    }
    pub fn key(&self) -> serde_json::Value {
        if self.echo() {
            serde_json::json!({"concat": [self.tuple.source.to_string(), self.tuple.destination.to_string(), self.tuple.source_key, self.sequence]})
        } else {
            self.tuple.key()
        }
    }
    pub fn set(&self) -> &'static str {
        match (self.echo(), self.tuple.source.is_ipv4()) {
            (true, true) => "echo_quarantine",
            (false, true) => "quarantine",
            (true, false) => "echo_quarantine6",
            (false, false) => "quarantine6",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub id: u32,
    pub mark: u32,
    pub original: Tuple,
    pub reply: Tuple,
}

pub(super) async fn lookup(tuple: OriginalTuple) -> Result<Option<Entry>, ExitError> {
    let (protocol, source_key, destination_key) = tuple.fields();
    let wanted = Tuple {
        source: tuple.source,
        destination: tuple.destination,
        protocol,
        source_key,
        destination_key,
    };
    let entry = netlink::get(&wanted).await.map_err(Reason::from)?;
    // The kernel finds a tuple in either direction. An entry whose reply
    // direction equals `wanted` has no original tuple `wanted`.
    Ok(entry.filter(|entry| entry.original == wanted))
}

/// An empty result is not a CT confirmation: conntrack is created by the
/// first packet, not by adding an nft qualification element.
pub(super) async fn lookup_owned(
    tuple: OriginalTuple,
    mark: u32,
) -> Result<Option<Entry>, ExitError> {
    if !mark.is_power_of_two() {
        return Err(Reason::OwnershipConflict.into());
    }
    let entry = lookup(tuple).await?;
    if entry
        .as_ref()
        .is_some_and(|entry| entry.mark & mark != mark)
    {
        return Err(Reason::OwnershipConflict.into());
    }
    Ok(entry)
}

pub(super) async fn delete(
    tuple: OriginalTuple,
    entry: &Entry,
    mark: u32,
) -> Result<(), ExitError> {
    if mark == 0 || !mark.is_power_of_two() || entry.mark & mark != mark {
        return Err(Reason::OwnershipConflict.into());
    }
    let current = lookup(tuple).await?;
    let Some(current) = current else {
        return Ok(());
    };
    if current != *entry {
        return Err(Reason::OwnershipConflict.into());
    }
    // Deletion names the original tuple, default zone and the checked kernel
    // id: a replacement entry under the same tuple is never deleted. The gate
    // must already be closed. Absence, not the answer, proves retirement.
    let _deleted = netlink::delete(&current.original, current.id)
        .await
        .map_err(Reason::from)?;
    if lookup(tuple).await?.is_some() {
        return Err(Reason::ConntrackUnavailable.into());
    }
    Ok(())
}

/// Whether no entry of either family carries `mark`.
pub(super) async fn mark_unused(mark: u32) -> Result<bool, ExitError> {
    Ok(!netlink::any_marked(mark, mark)
        .await
        .map_err(Reason::from)?)
}

/// The mark-filtered dump this owner relies on is usable. No dumped entry is
/// treated as identity, authority or deletion evidence.
pub(super) async fn available() -> Result<(), ExitError> {
    netlink::any_marked(u32::MAX, u32::MAX)
        .await
        .map(|_| ())
        .map_err(|failure| Reason::from(failure).into())
}

/// Test setup: a foreign (mark 0) entry with the inverse reply tuple.
#[cfg(test)]
pub(super) async fn insert_foreign(tuple: OriginalTuple, timeout: u32) -> Result<(), ExitError> {
    let (protocol, source_key, destination_key) = tuple.fields();
    netlink::insert(
        &Tuple {
            source: tuple.source,
            destination: tuple.destination,
            protocol,
            source_key,
            destination_key,
        },
        0,
        timeout,
    )
    .await
    .map_err(|failure| Reason::from(failure).into())
}

/// Test setup: remove a foreign (mark 0) entry by its exact identity.
#[cfg(test)]
pub(super) async fn delete_foreign(tuple: OriginalTuple) -> Result<(), ExitError> {
    let entry = lookup(tuple).await?.ok_or(Reason::ConntrackUnavailable)?;
    if entry.mark != 0 {
        return Err(Reason::OwnershipConflict.into());
    }
    match netlink::delete(&entry.original, entry.id).await {
        Ok(netlink::Deleted::Deleted) => Ok(()),
        Ok(netlink::Deleted::Absent) => Err(Reason::ConntrackUnavailable.into()),
        Err(failure) => Err(Reason::from(failure).into()),
    }
}
