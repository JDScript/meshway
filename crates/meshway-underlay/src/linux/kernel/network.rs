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
        address::{AddressAttribute, AddressMessage, AddressScope},
        link::LinkFlags,
        route::{RouteAddress, RouteAttribute, RouteMessage, RouteType},
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
/// close together is one change: it is classified once no notification came
/// for this long (trailing), and at the latest [`COALESCE_LIMIT`] after its
/// first notification, so a steady stream of events cannot postpone it.
pub(super) const COALESCE: Duration = Duration::from_millis(250);
pub(super) const COALESCE_LIMIT: Duration = Duration::from_secs(1);
/// One route lookup; a kernel that does not answer within it is no route.
const LOOKUP: Duration = Duration::from_secs(1);

/// The network observations every Host owner shares.
pub(in crate::linux) struct Network {
    pub changes: watch::Sender<NetworkChange>,
    /// Marks of external kernel notifications not yet classified.
    pub dirty: watch::Sender<u64>,
    /// The Server addresses (IPv4, IPv6) the control relay last reached.
    pub control: watch::Sender<[Option<IpAddr>; 2]>,
    /// The remotes connectivity currently sends to (channel remotes and
    /// literal dial candidates), at most [`MAX_REMOTES`].
    pub remotes: watch::Sender<BTreeSet<IpAddr>>,
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
            remotes: watch::channel(BTreeSet::new()).0,
            overlap: watch::channel(None).0,
            repairs: watch::channel(0).0,
            routes: None,
        }
    }
}

impl Network {
    /// The remotes connectivity sends to now, in its priority order; only
    /// the first [`MAX_REMOTES`] distinct unicast addresses are followed.
    pub fn remotes_observed(&self, remotes: impl IntoIterator<Item = IpAddr>) {
        let mut followed = BTreeSet::new();
        for remote in remotes {
            if followed.len() == MAX_REMOTES {
                break;
            }
            if !(remote.is_unspecified() || remote.is_loopback() || remote.is_multicast()) {
                followed.insert(remote);
            }
        }
        self.remotes
            .send_if_modified(|known| (*known != followed).then(|| *known = followed).is_some());
    }

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
    /// The destination is an address of this host (the local table).
    pub local: bool,
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
        local: found.header.kind == RouteType::Local,
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
    /// Their effective addresses ([`effective`]).
    addresses: BTreeSet<(u32, IpAddr, u8)>,
    /// The marked route toward each reference destination: the Server
    /// addresses the relay reached and the remotes connectivity sends to.
    routes: BTreeMap<IpAddr, Option<Route>>,
}

impl Fingerprint {
    fn summary(&self) -> (usize, usize, usize) {
        (self.links.len(), self.addresses.len(), self.routes.len())
    }

    /// Whether `current` is another path than `self`. A destination only one
    /// side looked up (it became a reference in between) compares nothing.
    fn moved(&self, current: &Self) -> bool {
        self.links != current.links
            || self.addresses != current.addresses
            || current.routes.iter().any(|(destination, route)| {
                self.routes
                    .get(destination)
                    .is_some_and(|known| known != route)
            })
    }
}

/// IFA_F_DADFAILED, IFA_F_DEPRECATED, IFA_F_TENTATIVE: never carries traffic
/// as a source in either family.
const UNUSABLE: u32 = 0x08 | 0x20 | 0x40;
/// Bit 0x01 is IFA_F_TEMPORARY for IPv6 (a privacy address, rotated daily)
/// but IFA_F_SECONDARY for IPv4 (a further address of the same prefix, as
/// effective as the primary).
const TEMPORARY_OR_SECONDARY: u32 = 0x01;

/// An address's flags: the extended `IFA_FLAGS` attribute when present,
/// otherwise the legacy header byte. Owners interpret the bits.
pub(super) fn address_flags(address: &AddressMessage) -> u32 {
    address
        .attributes
        .iter()
        .find_map(|attribute| match attribute {
            AddressAttribute::Flags(flags) => Some(flags.bits()),
            _ => None,
        })
        .unwrap_or(u32::from(address.header.flags.bits()))
}

/// An address that carries this Node's traffic as a source: usable, and not
/// an IPv6 temporary address. Flags are read per address family.
fn effective(address: IpAddr, flags: u32) -> bool {
    flags & UNUSABLE == 0 && !(address.is_ipv6() && flags & TEMPORARY_OR_SECONDARY != 0)
}

/// Remotes one fingerprint follows besides the Server, at most.
pub(in crate::linux) const MAX_REMOTES: usize = 64;

/// The marked routes toward `destinations`, looked up together.
async fn routes(
    handle: &Handle,
    destinations: impl IntoIterator<Item = IpAddr>,
    mark: u32,
) -> BTreeMap<IpAddr, Option<Route>> {
    let destinations: BTreeSet<_> = destinations.into_iter().collect();
    let found = futures_util::future::join_all(
        destinations
            .iter()
            .map(|destination| route(handle, *destination, mark)),
    )
    .await;
    destinations.into_iter().zip(found).collect()
}

/// Every destination whose route is part of the fingerprint.
fn references(control: [Option<IpAddr>; 2], remotes: &BTreeSet<IpAddr>) -> BTreeSet<IpAddr> {
    control
        .into_iter()
        .flatten()
        .chain(remotes.iter().copied())
        .collect()
}

async fn fingerprint(
    handle: &Handle,
    expected: &Mutex<monitor::Expected>,
    destinations: BTreeSet<IpAddr>,
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
        let flags = address_flags(entry);
        if !print.links.contains_key(&entry.header.index)
            || entry.header.scope != AddressScope::Universe
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
        if let Some(ip) = ip
            && effective(ip, flags)
        {
            print
                .addresses
                .insert((entry.header.index, ip, entry.header.prefix_len));
        }
    }
    print.routes = routes(handle, destinations, mark).await;
    Ok(print)
}

/// Wait out a burst that started with the notification just seen: until
/// [`COALESCE`] passed without another, or [`COALESCE_LIMIT`] since its
/// start. `false` once the notifications' sender is gone.
async fn settled(dirty: &mut watch::Receiver<u64>) -> bool {
    let start = tokio::time::Instant::now();
    let mut last = start;
    dirty.borrow_and_update();
    loop {
        let due = (last + COALESCE).min(start + COALESCE_LIMIT);
        tokio::select! {
            () = tokio::time::sleep_until(due) => return true,
            changed = dirty.changed() => {
                if changed.is_err() {
                    return false;
                }
                dirty.borrow_and_update();
                last = tokio::time::Instant::now();
            }
        }
    }
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
    let mut remotes = network.remotes.subscribe();
    let mark = shared.mark.value;
    let destinations = {
        let reached = *control.borrow_and_update();
        references(reached, &remotes.borrow_and_update())
    };
    let mut last = fingerprint(&handle, &expected, destinations, mark, limit)
        .await
        .ok();
    loop {
        tokio::select! { biased;
            changed = dirty.changed() => if changed.is_err() { return },
            // A new reference (a Server address the relay newly reached, a
            // remote connectivity now sends to) is looked up into the
            // current baseline; the routes already in it keep their values,
            // so a change that raced it is still compared.
            changed = control.changed() => {
                if changed.is_err() { return }
                let reached = *control.borrow_and_update();
                let current = remotes.borrow().clone();
                rebase(&handle, &mut last, reached, &current, mark).await;
                continue;
            }
            changed = remotes.changed() => {
                if changed.is_err() { return }
                let current = remotes.borrow_and_update().clone();
                let reached = *control.borrow();
                rebase(&handle, &mut last, reached, &current, mark).await;
                continue;
            }
        }
        if !settled(&mut dirty).await {
            return;
        }
        let destinations = {
            let reached = *control.borrow();
            references(reached, &remotes.borrow())
        };
        let current = fingerprint(&handle, &expected, destinations, mark, limit)
            .await
            .ok();
        let class = match (&last, &current) {
            (Some(last), Some(current)) if !last.moved(current) => NetworkChangeClass::Refresh,
            _ => NetworkChangeClass::Path,
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

/// Make the baseline's routes exactly those of the current references:
/// new destinations are looked up now, departed ones dropped.
async fn rebase(
    handle: &Handle,
    last: &mut Option<Fingerprint>,
    control: [Option<IpAddr>; 2],
    remotes: &BTreeSet<IpAddr>,
    mark: u32,
) {
    let Some(last) = last else {
        return;
    };
    let wanted = references(control, remotes);
    last.routes
        .retain(|destination, _| wanted.contains(destination));
    let added: Vec<_> = wanted
        .into_iter()
        .filter(|destination| !last.routes.contains_key(destination))
        .collect();
    last.routes.extend(routes(handle, added, mark).await);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F6a C6: a burst settles 250 ms after its last notification, not 250 ms
    /// after its first; a stream that never pauses settles 1 s after it began.
    #[tokio::test(start_paused = true)]
    async fn a_burst_settles_after_its_last_notification_within_a_bound() {
        let (sender, mut dirty) = watch::channel(0u64);
        let started = tokio::time::Instant::now();
        let feed = tokio::spawn({
            let sender = sender.clone();
            async move {
                // Spread across two fixed 250 ms windows, 200 ms apart.
                for _ in 0..2 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    sender.send_modify(|count| *count += 1);
                }
            }
        });
        assert!(settled(&mut dirty).await);
        assert_eq!(started.elapsed(), Duration::from_millis(650));
        feed.await.unwrap();
        assert!(
            !dirty.has_changed().unwrap(),
            "one burst, one classification"
        );

        let started = tokio::time::Instant::now();
        let feed = tokio::spawn({
            let sender = sender.clone();
            async move {
                for _ in 0..20 {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    sender.send_modify(|count| *count += 1);
                }
            }
        });
        assert!(settled(&mut dirty).await);
        assert_eq!(started.elapsed(), COALESCE_LIMIT);
        feed.abort();
        let _ = feed.await;
        drop(sender);
        assert!(!settled(&mut dirty).await);
    }

    #[test]
    fn address_flags_are_read_per_family() {
        let v4: IpAddr = "192.0.2.7".parse().unwrap();
        let v6: IpAddr = "2001:db8::7".parse().unwrap();
        // IFA_F_SECONDARY: a further IPv4 address of the prefix, effective.
        assert!(effective(v4, 0x01));
        assert!(effective(v4, 0x80));
        // IFA_F_TEMPORARY: an IPv6 privacy address, rotated; not followed.
        assert!(!effective(v6, 0x01));
        assert!(effective(v6, 0x80));
        for unusable in [0x08, 0x20, 0x40] {
            assert!(!effective(v4, unusable));
            assert!(!effective(v6, unusable));
        }
    }

    fn route(interface: u32, source: &str) -> Option<Route> {
        Some(Route {
            local: false,
            interface: Some(interface),
            gateway: None,
            source: Some(source.parse().unwrap()),
        })
    }

    #[test]
    fn a_route_of_any_family_toward_any_reference_moves_the_path() {
        let server: IpAddr = "2001:db8::443".parse().unwrap();
        let peer: IpAddr = "198.51.100.9".parse().unwrap();
        let base = Fingerprint {
            links: BTreeMap::from([(2, true)]),
            addresses: BTreeSet::from([(2, "192.0.2.7".parse().unwrap(), 24)]),
            routes: BTreeMap::from([
                (server, route(2, "2001:db8::7")),
                (peer, route(2, "192.0.2.7")),
            ]),
        };
        assert!(!base.moved(&base.clone()));
        // The Server's IPv6 route is unchanged; the IPv4 source toward the
        // peer moved to the second address: another path.
        let mut moved = base.clone();
        moved.routes.insert(peer, route(2, "192.0.2.8"));
        assert!(base.moved(&moved));
        // A destination that became a reference in between compares nothing.
        let mut added = base.clone();
        added
            .routes
            .insert("203.0.113.1".parse().unwrap(), route(3, "192.0.2.7"));
        assert!(!base.moved(&added));
        let mut gone = base.clone();
        gone.routes.remove(&peer);
        assert!(!base.moved(&gone));
        let mut secondary = base.clone();
        secondary
            .addresses
            .insert((2, "192.0.2.8".parse().unwrap(), 24));
        assert!(base.moved(&secondary));
    }

    #[test]
    fn observed_remotes_are_bounded_unicast_and_published_on_change() {
        let network = Network::default();
        let mut remotes = network.remotes.subscribe();
        network.remotes_observed(
            [
                "0.0.0.0",
                "127.0.0.1",
                "224.0.0.1",
                "192.0.2.1",
                "192.0.2.1",
            ]
            .map(|ip| ip.parse().unwrap()),
        );
        assert!(remotes.has_changed().unwrap());
        assert_eq!(
            *remotes.borrow_and_update(),
            BTreeSet::from(["192.0.2.1".parse().unwrap()])
        );
        network.remotes_observed(["192.0.2.1".parse().unwrap()]);
        assert!(!remotes.has_changed().unwrap());
        network.remotes_observed(
            (0..200u32).map(|n| IpAddr::V4(std::net::Ipv4Addr::from(0xc633_6400 + n))),
        );
        assert_eq!(remotes.borrow_and_update().len(), MAX_REMOTES);
    }

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
