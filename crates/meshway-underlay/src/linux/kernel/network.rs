//! Classification of external network changes, the one Underlay reaction to
//! the kernel monitor's marks: after a burst of external notifications
//! settles, one fingerprint decides between a path change and a refresh.
//! Routes are looked up through the real RPDB with this Node's own mark, so
//! policy tables other than main count as they would for its sockets.
use crate::{NetworkChange, NetworkChangeClass, PoolOverlap};
use rtnetlink::{
    Handle,
    packet_route::{
        AddressFamily,
        address::{AddressAttribute, AddressScope},
        link::LinkFlags,
        route::{RouteAddress, RouteAttribute, RouteMessage},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;

use super::{super::protection::Shared, collect, monitor};

/// A burst of notifications (DHCP, router advertisements, a link flap) this
/// close together is one change.
pub(super) const COALESCE: Duration = Duration::from_millis(250);
/// One route lookup; a kernel that does not answer within it is no route.
const LOOKUP: Duration = Duration::from_secs(1);

/// The network observations every Host owner shares.
pub(in crate::linux) struct Network {
    pub changes: watch::Sender<NetworkChange>,
    /// Marks of external kernel notifications not yet classified.
    pub dirty: watch::Sender<u64>,
    /// The Server addresses (IPv4, IPv6) the control relay last reached.
    pub control: watch::Sender<[Option<IpAddr>; 2]>,
    /// The first external address inside a Mesh pool, while one is assigned.
    pub overlap: watch::Sender<Option<PoolOverlap>>,
    /// Owned capture objects removed outside their owner, to reinstall.
    pub repairs: watch::Sender<u64>,
    /// Route lookups; none in unit fixtures without a kernel owner.
    pub routes: Option<Handle>,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            changes: watch::channel(NetworkChange::default()).0,
            dirty: watch::channel(0).0,
            control: watch::channel([None, None]).0,
            overlap: watch::channel(None).0,
            repairs: watch::channel(0).0,
            routes: None,
        }
    }
}

impl Network {
    /// The control relay reached `address`.
    pub fn control_reached(&self, address: IpAddr) {
        self.control.send_if_modified(|known| {
            let slot = &mut known[usize::from(address.is_ipv6())];
            (*slot != Some(address))
                .then(|| *slot = Some(address))
                .is_some()
        });
    }
}

/// The route this Node's marked sockets take toward one destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub interface: Option<u32>,
    pub gateway: Option<IpAddr>,
    pub source: Option<IpAddr>,
}

fn address(value: &RouteAddress) -> Option<IpAddr> {
    match value {
        RouteAddress::Inet(ip) => Some(IpAddr::V4(*ip)),
        RouteAddress::Inet6(ip) => Some(IpAddr::V6(*ip)),
        _ => None,
    }
}

/// One RTM_GETROUTE toward `destination` with `mark`, through the full
/// RPDB; `None` when the kernel has no route (unreachable, prohibited).
pub(in crate::linux) async fn route(
    handle: &Handle,
    destination: IpAddr,
    mark: u32,
) -> Option<Route> {
    let mut message = RouteMessage::default();
    let (family, length, target) = match destination {
        IpAddr::V4(ip) => (AddressFamily::Inet, 32, RouteAddress::Inet(ip)),
        IpAddr::V6(ip) => (AddressFamily::Inet6, 128, RouteAddress::Inet6(ip)),
    };
    message.header.address_family = family;
    message.header.destination_prefix_length = length;
    message.attributes.push(RouteAttribute::Destination(target));
    message.attributes.push(RouteAttribute::Mark(mark));
    let found = tokio::time::timeout(LOOKUP, async {
        use futures_util::TryStreamExt;
        handle.route().get(message).execute().try_next().await
    })
    .await;
    let Ok(Ok(Some(found))) = found else {
        return None;
    };
    let mut route = Route {
        interface: None,
        gateway: None,
        source: None,
    };
    for attribute in &found.attributes {
        match attribute {
            RouteAttribute::Oif(index) => route.interface = Some(*index),
            RouteAttribute::Gateway(value) => route.gateway = address(value),
            RouteAttribute::PrefSource(value) => route.source = address(value),
            _ => {}
        }
    }
    Some(route)
}

/// What decides whether peers now see this Node from another path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Fingerprint {
    /// External up interfaces and whether each has carrier.
    links: BTreeMap<u32, bool>,
    /// Their preferred addresses: global, neither temporary, deprecated,
    /// tentative nor failed duplicate detection.
    addresses: BTreeSet<(u32, IpAddr, u8)>,
    /// The marked route toward each Server address the relay reached.
    routes: Vec<(IpAddr, Option<Route>)>,
}

impl Fingerprint {
    fn summary(&self) -> (usize, usize, usize) {
        (self.links.len(), self.addresses.len(), self.routes.len())
    }
}

/// IFA_F_TEMPORARY, IFA_F_DADFAILED, IFA_F_DEPRECATED, IFA_F_TENTATIVE.
const UNPREFERRED: u32 = 0x01 | 0x08 | 0x20 | 0x40;

async fn fingerprint(
    handle: &Handle,
    expected: &Mutex<monitor::Expected>,
    control: [Option<IpAddr>; 2],
    mark: u32,
    limit: usize,
) -> crate::Result<Fingerprint> {
    let links = collect(handle.link().get().execute(), limit).await?;
    let addresses = collect(handle.address().get().execute(), limit).await?;
    let mut print = Fingerprint::default();
    {
        let state = expected.lock().map_err(|_| crate::Error::Worker)?;
        for link in &links {
            if link.header.flags.contains(LinkFlags::Up)
                && monitor::external(&state, link.header.index)
            {
                print.links.insert(
                    link.header.index,
                    link.header.flags.contains(LinkFlags::LowerUp),
                );
            }
        }
    }
    for entry in &addresses {
        let flags = entry
            .attributes
            .iter()
            .find_map(|attribute| match attribute {
                AddressAttribute::Flags(flags) => Some(flags.bits()),
                _ => None,
            })
            .unwrap_or(u32::from(entry.header.flags.bits()));
        if !print.links.contains_key(&entry.header.index)
            || entry.header.scope != AddressScope::Universe
            || flags & UNPREFERRED != 0
        {
            continue;
        }
        let ip = entry
            .attributes
            .iter()
            .find_map(|attribute| match attribute {
                AddressAttribute::Local(ip) => Some(*ip),
                _ => None,
            });
        let ip = ip.or_else(|| {
            entry
                .attributes
                .iter()
                .find_map(|attribute| match attribute {
                    AddressAttribute::Address(ip) => Some(*ip),
                    _ => None,
                })
        });
        if let Some(ip) = ip {
            print
                .addresses
                .insert((entry.header.index, ip, entry.header.prefix_len));
        }
    }
    for destination in control.into_iter().flatten() {
        print
            .routes
            .push((destination, route(handle, destination, mark).await));
    }
    Ok(print)
}

/// Classify every settled burst of external notifications. A fingerprint
/// that cannot be read counts as a path change: reacting once too often is
/// bounded, missing a roam is not.
pub(super) async fn classify(
    shared: Arc<Shared>,
    handle: Handle,
    expected: Arc<Mutex<monitor::Expected>>,
    limit: usize,
) {
    let network = &shared.network;
    let mut dirty = network.dirty.subscribe();
    let mut control = network.control.subscribe();
    let mark = shared.mark.value;
    let reached = *control.borrow_and_update();
    let mut last = fingerprint(&handle, &expected, reached, mark, limit)
        .await
        .ok();
    loop {
        tokio::select! { biased;
            changed = dirty.changed() => if changed.is_err() { return },
            // A newly reached Server address is a new reference, not a change.
            changed = control.changed() => {
                if changed.is_err() { return }
                let reached = *control.borrow_and_update();
                last = fingerprint(&handle, &expected, reached, mark, limit).await.ok();
                continue;
            }
        }
        tokio::time::sleep(COALESCE).await;
        dirty.borrow_and_update();
        let reached = *control.borrow();
        let current = fingerprint(&handle, &expected, reached, mark, limit)
            .await
            .ok();
        let class = if current.is_none() || current != last {
            NetworkChangeClass::Path
        } else {
            NetworkChangeClass::Refresh
        };
        let summary = current.as_ref().map(Fingerprint::summary);
        last = current;
        network.changes.send_modify(|change| {
            change.epoch = change.epoch.wrapping_add(1);
            if class == NetworkChangeClass::Path {
                change.path = change.path.wrapping_add(1);
            }
            change.class = Some(class);
            change.at = Some(std::time::SystemTime::now());
            tracing::debug!(
                class = ?class,
                network_epoch = change.epoch,
                path_epoch = change.path,
                fingerprint = ?summary,
                "Underlay network change classified"
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newly_reached_server_address_is_recorded_once_per_family() {
        let network = Network::default();
        let mut control = network.control.subscribe();
        network.control_reached("192.0.2.1".parse().unwrap());
        assert!(control.has_changed().unwrap());
        control.borrow_and_update();
        network.control_reached("192.0.2.1".parse().unwrap());
        assert!(!control.has_changed().unwrap());
        network.control_reached("2001:db8::1".parse().unwrap());
        assert_eq!(
            *control.borrow_and_update(),
            [
                Some("192.0.2.1".parse().unwrap()),
                Some("2001:db8::1".parse().unwrap())
            ]
        );
    }
}
