//! Persistent DNS anchor, owned by the existing Kernel journal and monitor.
//! It has a local installation address, never a leased Server Mesh address.
use super::{
    Kernel, Pending, collect, execute, get_addresses, get_links, get_routes, rtnl::Handle,
};
use crate::{Error, Result};
use meshway_dns::{DnsFuture, DnsLinkIdentity, OwnedDnsLink, SystemDnsError, SystemDnsFailure};
use rtnetlink::{
    LinkDummy,
    packet_route::{
        AddressFamily,
        address::{AddressAttribute, AddressMessage, AddressScope},
        link::{
            AfSpecInet6, AfSpecUnspec, In6AddrGenMode, InfoKind, LinkAttribute, LinkFlags,
            LinkInfo, LinkMessage,
        },
        route::{RouteAddress, RouteAttribute, RouteMessage},
    },
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub identity: DnsLinkIdentity,
    pub alias: String,
    pub hardware_address: Vec<u8>,
    pub mtu: u32,
}

/// What a DNS anchor creation gives its device before the kernel sees it:
/// the journal records it with the creation's intent, so a later term can
/// tell the device this owner created (its random hardware address) from
/// any other with the same name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Creation {
    pub name: String,
    pub hardware_address: Vec<u8>,
    pub mtu: u32,
}

/// The device a recorded creation left, by its fingerprint: `Ok(Some(index))`
/// is the one this owner created (its name, random hardware address, MTU,
/// a dummy without a controller, and no alias but its own); `Ok(None)` none
/// carries its name or alias; anything else with them is not ours
/// (`Journal`, as before: never claimed by name).
pub(super) fn half_created(
    creation: &Creation,
    alias: &str,
    links: &[LinkMessage],
) -> Result<Option<u32>> {
    let named: Vec<_> = links
        .iter()
        .filter(|link| {
            link.attributes.iter().any(|a| {
                matches!(a, LinkAttribute::IfName(v) if v == &creation.name)
                    || matches!(a, LinkAttribute::IfAlias(v) if v == alias)
            })
        })
        .collect();
    let [link] = named.as_slice() else {
        return if named.is_empty() {
            Ok(None)
        } else {
            Err(Error::Journal)
        };
    };
    let ours = link.header.index != 0
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfName(v) if v == &creation.name))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Address(bytes) if bytes == &creation.hardware_address))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Mtu(mtu) if *mtu == creation.mtu))
        && !link.attributes.iter().any(|a| matches!(a, LinkAttribute::Controller(index) if *index != 0))
        && !link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfAlias(v) if !v.is_empty() && v != alias))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::LinkInfo(infos) if infos.iter().any(|info| matches!(info, LinkInfo::Kind(InfoKind::Dummy)))));
    if ours {
        Ok(Some(link.header.index))
    } else {
        Err(Error::Journal)
    }
}

/// Remove the device a creation interrupted in an ended term or process
/// left, identified by its recorded fingerprint ([`half_created`]); the DNS
/// owner creates the anchor again on the new term. A read or deletion that
/// fails proves nothing (the re-proof retries it); a device of its name
/// that is not ours stays, and the journal with it.
pub(super) async fn reclaim(
    handle: &Handle,
    creation: &Creation,
    alias: &str,
    limit: usize,
) -> Result<()> {
    let Some(index) = half_created(creation, alias, &get_links(handle, limit).await?)? else {
        return Ok(());
    };
    execute(handle.link().del(index).execute()).await?;
    if half_created(creation, alias, &get_links(handle, limit).await?)?.is_some() {
        return Err(Error::Conflict);
    }
    tracing::info!(
        index,
        name = creation.name,
        "Half-created DNS anchor of an interrupted creation reclaimed by its fingerprint"
    );
    Ok(())
}

struct Guard {
    /// The read-only verifier's own request socket. Cloning Kernel.handle
    /// would share its multipart-dump slot with mutations and their
    /// verification. A read that does not complete closes it and the next
    /// read opens another: an interrupted read proves nothing about the
    /// anchor, so it never ends the anchor itself.
    reader: tokio::sync::Mutex<Option<Reader>>,
    shared: Arc<super::super::protection::Shared>,
    /// For the source-capture table, which the collision check exempts.
    expected: Arc<std::sync::Mutex<super::monitor::Expected>>,
    record: Record,
    limit: usize,
    /// Ends the anchor: its owner removed it (`remove_dns_anchor`) or
    /// dropped this guard.
    cancelled: CancellationToken,
}
#[derive(Clone)]
struct Reader {
    handle: Handle,
    /// Closes this socket (its connection task ends), never the anchor.
    closed: CancellationToken,
}
impl Guard {
    /// Whether the anchor still holds. The end of kernel ownership is judged
    /// by the Host, not here: an unavailable end proves nothing about the
    /// anchor (`Backend`; the Host re-proves it and DNS prepares again on
    /// the new term), only a proven conflict or the anchor's own removal is
    /// one (`Conflict`).
    fn check(&self) -> std::result::Result<(), SystemDnsFailure> {
        let (error, actual) = if let Some((cause, _)) = self.shared.ended() {
            match cause {
                crate::OwnershipEnd::Unavailable { .. } => (
                    SystemDnsError::Backend,
                    "the host's kernel ownership ended unavailable (the Host proves it again; DNS prepares again on its next term)",
                ),
                crate::OwnershipEnd::Conflict { .. } => (
                    SystemDnsError::Conflict,
                    "the host's kernel ownership ended on a conflict (its owner logged why)",
                ),
            }
        } else if self.shared.tasks.stop.is_cancelled() {
            (SystemDnsError::Backend, "the host owner stopped")
        } else if self.cancelled.is_cancelled() {
            (
                SystemDnsError::Conflict,
                "the anchor was removed or its guard dropped",
            )
        } else {
            return Ok(());
        };
        Err(SystemDnsFailure::new(error, "anchor_ended")
            .expected(format!(
                "anchor {} owned by this Kernel",
                self.record.identity.name
            ))
            .actual(actual)
            .source("Kernel DNS anchor"))
    }
    /// A new request socket and its connection task, bounded by the host's
    /// task capacity.
    fn open_reader(&self) -> Result<Reader> {
        let closed = self.cancelled.child_token();
        let (connection, handle) = super::rtnl::connect()?;
        let stop = closed.clone();
        let conflict = self.shared.conflict.clone();
        // Tasks rejects at its existing capacity bound. On rejection the
        // supplied future (and its newly allocated socket) is dropped here.
        self.shared.tasks.spawn(async move {
            tokio::select! { biased;
                _ = stop.cancelled() => {},
                _ = conflict.cancelled() => {},
                () = connection => { stop.cancel(); },
            }
        })?;
        Ok(Reader { handle, closed })
    }
    fn failure(&self, mismatch: Mismatch) -> SystemDnsFailure {
        // Ended while verifying: why it ended is the guard's own judgement.
        if mismatch.check == "anchor_ended"
            && let Err(ended) = self.check()
        {
            return ended;
        }
        let identity = &self.record.identity;
        // A dump that failed read nothing back: it proves nothing about the
        // anchor (`Backend`, re-proved); only a read-back that differs is one.
        let error = if mismatch.error.judgement() == crate::Judgement::Unknown {
            SystemDnsError::Backend
        } else {
            SystemDnsError::Conflict
        };
        SystemDnsFailure::new(error, mismatch.check)
            .expected(format!(
                "dummy {} index {} up with only {}/32 (alias {}), no other owner of that address",
                identity.name, identity.index, identity.address, self.record.alias
            ))
            .actual(mismatch.actual)
            .source(mismatch.source)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // This is a child of the anchor's token, never the Kernel's epoch.
        self.cancelled.cancel();
    }
}

// Dropping a response stream does not cancel the kernel's multipart dump. An
// interrupted or failed read closes its reader before another caller can use
// its FD. A caller cancelled while still waiting for the mutex owns no dump.
struct Reading(Option<CancellationToken>);
impl Drop for Reading {
    fn drop(&mut self) {
        if let Some(closed) = &self.0 {
            closed.cancel();
        }
    }
}
impl OwnedDnsLink for Guard {
    fn verify(&self) -> DnsFuture<'_, std::result::Result<DnsLinkIdentity, SystemDnsFailure>> {
        Box::pin(async {
            self.check()?;
            // GET execute() enqueues eagerly, so lock before invoking even the
            // first verifier helper, not inside collect(response_stream).
            let mut slot = tokio::select! { biased;
                _ = self.ended() => return Err(self.check().err().unwrap_or_else(|| self.failure(Mismatch::ended()))),
                slot = self.reader.lock() => slot,
            };
            self.check()?;
            let reader = match slot.as_ref().filter(|reader| !reader.closed.is_cancelled()) {
                Some(reader) => reader.clone(),
                None => {
                    let reader = self.open_reader().map_err(|error| {
                        SystemDnsFailure::new(SystemDnsError::Backend, "anchor_reader")
                            .actual(error.to_string())
                            .source("rtnetlink socket")
                    })?;
                    *slot = Some(reader.clone());
                    reader
                }
            };
            let mut reading = Reading(Some(reader.closed.clone()));
            let capture = self
                .expected
                .lock()
                .map_err(|_| self.failure(Mismatch::ended()))?
                .egress_table;
            tokio::select! { biased;
                _ = self.ended() => return Err(self.check().err().unwrap_or_else(|| self.failure(Mismatch::ended()))),
                result = inspect(&reader.handle, &self.record, capture, self.limit) => result.map_err(|mismatch| self.failure(mismatch))?,
            }
            reading.0 = None;
            self.check()?;
            Ok(self.record.identity.clone())
        })
    }
    fn ended(&self) -> DnsFuture<'_, ()> {
        Box::pin(async {
            tokio::select! {
                _ = self.shared.conflict.cancelled() => {},
                _ = self.shared.tasks.stop.cancelled() => {},
                _ = self.cancelled.cancelled() => {},
            }
        })
    }
}
impl Kernel {
    /// Acquire/create the DNS anchor only after the namespace-wide DNS lock is
    /// held by the host installer. Existing records are verified, never adopted
    /// by name alone. DNS settings must be removed before remove_dns_anchor.
    pub async fn dns_anchor(
        &mut self,
        reservation: &meshway_dns::SystemDnsReservation,
    ) -> Result<Arc<dyn OwnedDnsLink>> {
        self.ready().await?;
        self.verify().await?;
        reservation
            .verify_namespace(
                &self.journal.boot,
                self.journal.namespace_device,
                self.journal.namespace_inode,
                self.journal.namespace_cookie,
            )
            .map_err(|_| Error::Conflict)?;
        if !matches!(
            reservation.settings().backend,
            meshway_dns::SystemDnsBackend::Resolved
        ) {
            return Err(Error::Options);
        }
        // Configuration refusals: no kernel object was read for them.
        let address = reservation.settings().anchor_address;
        if address.is_unspecified()
            || address.is_multicast()
            || address.is_loopback()
            || address.is_link_local()
            || address.is_broadcast()
        {
            return Err(Error::Options);
        }
        if self
            .journal
            .pools
            .iter()
            .any(|pool| pool.contains(&IpAddr::V4(address)))
        {
            return Err(Error::AnchorInPool { address });
        }
        if let Some(record) = &self.journal.anchor {
            // The settings name another address than the anchor this host
            // holds (removed only by `down`).
            if record.identity.address != address {
                return Err(Error::Options);
            }
        } else {
            let name = self.journal.anchor_name();
            let alias = self.journal.anchor_alias();
            let links = get_links(&self.handle, self.options.max_kernel_objects).await?;
            if links.iter().any(|link| {
                link.attributes.iter().any(|a| {
                    matches!(a, LinkAttribute::IfName(v) if v == &name)
                        || matches!(a, LinkAttribute::IfAlias(v) if v == &alias)
                })
            }) {
                return Err(Error::Conflict);
            }
            let capture = self
                .expected
                .lock()
                .map_err(|_| Error::Worker)?
                .egress_table;
            collision(
                &self.handle,
                address,
                None,
                capture,
                self.options.max_kernel_objects,
            )
            .await?;
            // IFLA_IFALIAS is not installed by the generic dummy NEWLINK path.
            // Give creation its own known fingerprint so the subsequent SETLINK
            // never adopts a same-name device with an unknown identity.
            let mut hardware_address = vec![0u8; 6];
            getrandom::fill(&mut hardware_address).map_err(|_| Error::Worker)?;
            hardware_address[0] = (hardware_address[0] & 0xfe) | 0x02;
            let mtu = 1500;
            // The device's own notifications are not an external network
            // change, from its first NEWLINK (by name) until the steady-state
            // record below owns its index.
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .anchor_creating = Some((name.clone(), None));
            let created = async {
                // The fingerprint is durable before the device exists: a
                // later term reclaims exactly this device (`reclaim`).
                self.journal.pending = Some(Pending::CreateDnsAnchor {
                    device: Some(Creation {
                        name: name.clone(),
                        hardware_address: hardware_address.clone(),
                        mtu,
                    }),
                });
                self.store.persist(&self.journal).await?;
                // rtnl_create_link validates AF_SPEC before register_netdevice;
                // inet6_validate_link_af then rejects the not-yet-created inet6_dev
                // with EAFNOSUPPORT. Register DOWN first, without AF_SPEC or alias.
                // NETDEV_REGISTER creates inet6_dev; autoconfiguration waits for UP.
                let link = LinkDummy::new(&name)
                    .address(hardware_address.clone())
                    .mtu(mtu)
                    .down()
                    .build();
                execute(self.handle.link().add(link).execute()).await?;
                #[cfg(test)]
                if super::tool::command::faults::take(
                    &super::tool::command::faults::FAILING_ANCHOR_READS,
                ) {
                    return Err(Error::Netlink);
                }
                let links = collect(
                    self.handle.link().get().match_name(name.clone()).execute(),
                    self.options.max_kernel_objects,
                )
                .await?;
                if links.len() != 1 {
                    return Err(Error::Conflict);
                }
                let index = links[0].header.index;
                self.expected
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .anchor_creating = Some((name.clone(), Some(index)));
                let record = Record {
                    identity: DnsLinkIdentity {
                        boot: self.journal.boot.trim().to_owned(),
                        namespace_device: self.journal.namespace_device,
                        namespace_inode: self.journal.namespace_inode,
                        installation: self.journal.installation.clone(),
                        index,
                        name,
                        address,
                    },
                    alias,
                    hardware_address,
                    mtu,
                };
                if index == 0
                    || !device_matches(&record, &links[0])
                    || links[0].header.flags.contains(LinkFlags::Up)
                    || links[0]
                        .attributes
                        .iter()
                        .any(|a| matches!(a, LinkAttribute::IfAlias(value) if !value.is_empty()))
                    || get_addresses(&self.handle, self.options.max_kernel_objects)
                        .await?
                        .iter()
                        .any(|address| address.header.index == index)
                {
                    return Err(Error::Conflict);
                }
                // This separate SETLINK runs against a registered, fingerprinted,
                // still-DOWN device. Apply/read back both alias and addrgenmode
                // before any transition that could generate an IPv6 address.
                let configured = rtnetlink::LinkUnspec::new_with_index(index)
                    .alias(record.alias.clone())
                    .append_extra_attribute(LinkAttribute::AfSpecUnspec(vec![AfSpecUnspec::Inet6(
                        vec![AfSpecInet6::AddrGenMode(In6AddrGenMode::None)],
                    )]))
                    .build();
                execute(self.handle.link().set(configured).execute()).await?;
                let configured = owned_link(&record, self.options.max_kernel_objects).await?;
                if configured.header.flags.contains(LinkFlags::Up)
                    || !installed_settings_match(&record, &configured)
                {
                    return Err(Error::Conflict);
                }
                let mut request = self.handle.address().add(index, IpAddr::V4(address), 32);
                request.message_mut().header.scope = AddressScope::Universe;
                execute(request.execute()).await?;
                let configured = owned_link(&record, self.options.max_kernel_objects).await?;
                if configured.header.flags.contains(LinkFlags::Up)
                    || !installed_settings_match(&record, &configured)
                {
                    return Err(Error::Conflict);
                }
                verify_addresses(&self.handle, &record, self.options.max_kernel_objects).await?;
                execute(
                    self.handle
                        .link()
                        .set(rtnetlink::LinkUnspec::new_with_index(index).up().build())
                        .execute(),
                )
                .await?;
                let capture = self
                    .expected
                    .lock()
                    .map_err(|_| Error::Worker)?
                    .egress_table;
                verify(
                    &self.handle,
                    &record,
                    capture,
                    self.options.max_kernel_objects,
                )
                .await?;
                // Drain the creation's down/address/up notifications before arming
                // the steady-state watcher, then close the dump/watch race by a
                // second verification after arming it.
                self.quiesce().await?;
                self.journal.anchor = Some(record.clone());
                self.expected.lock().map_err(|_| Error::Worker)?.anchor = Some(record);
                verify(
                    &self.handle,
                    self.journal.anchor.as_ref().ok_or(Error::Journal)?,
                    capture,
                    self.options.max_kernel_objects,
                )
                .await?;
                self.journal.pending = None;
                self.store.persist(&self.journal).await?;
                self.quiesce().await?;
                Ok(())
            }
            .await;
            self.expected
                .lock()
                .map_err(|_| Error::Worker)?
                .anchor_creating = None;
            created?;
            self.dns_cancel = CancellationToken::new();
        }
        let record = self.journal.anchor.clone().ok_or(Error::Journal)?;
        self.shared.check()?;
        let mut guard = Guard {
            reader: tokio::sync::Mutex::new(None),
            shared: self.shared.clone(),
            expected: self.expected.clone(),
            record,
            limit: self.options.max_kernel_objects,
            cancelled: self.dns_cancel.child_token(),
        };
        *guard.reader.get_mut() = Some(guard.open_reader()?);
        guard.check().map_err(|_| Error::Conflict)?;
        Ok(Arc::new(guard))
    }

    /// Caller has restored the exact owned system-DNS settings first. Suspend
    /// and process exit deliberately never invoke this method.
    pub async fn remove_dns_anchor(&mut self) -> Result<()> {
        self.ready().await?;
        self.verify().await?;
        let Some(record) = self.journal.anchor.clone() else {
            return Ok(());
        };
        self.journal.pending = Some(Pending::DeleteDnsAnchor);
        self.store.persist(&self.journal).await?;
        self.dns_cancel.cancel();
        self.expected
            .lock()
            .map_err(|_| Error::Worker)?
            .anchor_deleting = true;
        execute(self.handle.link().del(record.identity.index).execute()).await?;
        let links = get_links(&self.handle, self.options.max_kernel_objects).await?;
        if links.iter().any(|link| {
            link.header.index == record.identity.index
                || link.attributes.iter().any(
                    |a| matches!(a, LinkAttribute::IfName(name) if name == &record.identity.name),
                )
        }) {
            return Err(Error::Conflict);
        }
        self.quiesce().await?;
        {
            let mut expected = self.expected.lock().map_err(|_| Error::Worker)?;
            expected.anchor = None;
            expected.anchor_deleting = false;
        }
        self.journal.anchor = None;
        self.journal.pending = None;
        self.store.persist(&self.journal).await
    }
}
pub(super) fn link_matches(record: &Record, link: &LinkMessage) -> bool {
    deleting_link_matches(record, link)
        && link
            .header
            .flags
            .contains(LinkFlags::Up | LinkFlags::LowerUp)
}

/// RTM_DELLINK first reports the same configured device becoming DOWN. Only
/// the journalled deletion window may accept that flag transition; its exact
/// identity, alias and address-generation configuration must still match.
pub(super) fn deleting_link_matches(record: &Record, link: &LinkMessage) -> bool {
    device_matches(record, link)
        && !link
            .header
            .flags
            .intersects(LinkFlags::Loopback | LinkFlags::Dormant)
        && installed_settings_match(record, link)
}
fn device_matches(record: &Record, link: &LinkMessage) -> bool {
    link.header.index == record.identity.index
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Address(bytes) if bytes == &record.hardware_address))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::Mtu(mtu) if *mtu == record.mtu))
        && !link.attributes.iter().any(|a| matches!(a, LinkAttribute::Controller(index) if *index != 0))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::IfName(v) if v == &record.identity.name))
        && link.attributes.iter().any(|a| matches!(a, LinkAttribute::LinkInfo(infos) if infos.iter().any(|info| matches!(info, LinkInfo::Kind(InfoKind::Dummy)))))
}
fn installed_settings_match(record: &Record, link: &LinkMessage) -> bool {
    link.attributes
        .iter()
        .any(|a| matches!(a, LinkAttribute::IfAlias(value) if value == &record.alias))
        && link.attributes.iter().any(|a| {
            matches!(a,
            LinkAttribute::AfSpecUnspec(families) if families.iter().any(|family| matches!(family,
                AfSpecUnspec::Inet6(settings) if settings.iter().any(|setting| matches!(setting,
                    AfSpecInet6::AddrGenMode(In6AddrGenMode::None))))))
        })
}
pub(super) fn address_matches(record: &Record, address: &AddressMessage) -> bool {
    let flags = super::network::address_flags(address);
    address.header.index == record.identity.index && address.header.family == AddressFamily::Inet
        && address.header.prefix_len == 32 && address.header.scope == AddressScope::Universe && flags & 0x68 == 0
        && address.attributes.iter().any(|a| matches!(a, AddressAttribute::Local(IpAddr::V4(ip)) if *ip == record.identity.address))
        && address.attributes.iter().all(|a| match a {
            AddressAttribute::Address(ip) | AddressAttribute::Local(ip) => *ip == IpAddr::V4(record.identity.address),
            _ => true,
        })
}
pub(super) fn route_conflicts(address: Ipv4Addr, owned: Option<u32>, route: &RouteMessage) -> bool {
    if route.header.address_family != AddressFamily::Inet
        || route.header.destination_prefix_length == 0
    {
        return false;
    }
    let Some(destination) = route.attributes.iter().find_map(|a| match a {
        RouteAttribute::Destination(RouteAddress::Inet(ip)) => Some(*ip),
        _ => None,
    }) else {
        return true;
    };
    let prefix = route.header.destination_prefix_length;
    if prefix > 32 {
        return true;
    }
    let mask = u32::MAX << (32 - prefix);
    if u32::from(destination) & mask != u32::from(address) & mask {
        return false;
    }
    !(prefix == 32
        && destination == address
        && route.header.protocol == rtnetlink::packet_route::route::RouteProtocol::Kernel
        && ((super::route_table(route) == 255
            && route.header.kind == rtnetlink::packet_route::route::RouteType::Local)
            || (super::route_table(route) == 254
                && route.header.kind == rtnetlink::packet_route::route::RouteType::Unicast))
        && owned.is_some_and(|index| {
            route
                .attributes
                .iter()
                .any(|a| matches!(a, RouteAttribute::Oif(i) if *i == index))
        }))
}
/// Why the anchor failed a verification: the check, what was found and
/// which dump showed it. `error` is what Kernel callers see.
pub(super) struct Mismatch {
    error: Error,
    check: &'static str,
    actual: String,
    source: &'static str,
}
impl Mismatch {
    fn new(check: &'static str, actual: String, source: &'static str) -> Self {
        Self {
            error: Error::Conflict,
            check,
            actual,
            source,
        }
    }
    fn ended() -> Self {
        Self::new(
            "anchor_ended",
            "ended while verifying".into(),
            "Kernel DNS anchor",
        )
    }
    /// A dump that failed, timed out (5 s) or exceeded the object limit.
    fn dump(source: &'static str) -> impl Fn(Error) -> Self {
        move |error| Self {
            actual: format!("dump failed: {error}"),
            error,
            check: "anchor_netlink",
            source,
        }
    }
}
const LINKS: &str = "rtnetlink RTM_GETLINK";
const ADDRESSES: &str = "rtnetlink RTM_GETADDR";
const ROUTES: &str = "rtnetlink RTM_GETROUTE";

fn describe_link(link: &LinkMessage) -> String {
    let name = link.attributes.iter().find_map(|a| match a {
        LinkAttribute::IfName(name) => Some(name.as_str()),
        _ => None,
    });
    let mtu = link.attributes.iter().find_map(|a| match a {
        LinkAttribute::Mtu(mtu) => Some(*mtu),
        _ => None,
    });
    let dummy = link.attributes.iter().any(|a| matches!(a, LinkAttribute::LinkInfo(infos) if infos.iter().any(|info| matches!(info, LinkInfo::Kind(InfoKind::Dummy)))));
    format!(
        "index {} name {} mtu {} {} flags {:?}",
        link.header.index,
        name.unwrap_or("?"),
        mtu.map_or_else(|| "?".into(), |mtu| mtu.to_string()),
        if dummy { "dummy" } else { "not dummy" },
        link.header.flags
    )
}
fn describe_route(route: &RouteMessage) -> String {
    let destination = route.attributes.iter().find_map(|a| match a {
        RouteAttribute::Destination(RouteAddress::Inet(ip)) => Some(ip.to_string()),
        _ => None,
    });
    let oif = route.attributes.iter().find_map(|a| match a {
        RouteAttribute::Oif(index) => Some(*index),
        _ => None,
    });
    format!(
        "route {}/{} table {} dev index {} ({:?}, {:?})",
        destination.unwrap_or_else(|| "?".into()),
        route.header.destination_prefix_length,
        super::route_table(route),
        oif.map_or_else(|| "?".into(), |index| index.to_string()),
        route.header.kind,
        route.header.protocol
    )
}

/// `capture` is the source-capture table: only the capture selector looks it
/// up, after the local table, so its routes never steer the anchor address.
async fn collision(
    handle: &Handle,
    address: Ipv4Addr,
    owned: Option<u32>,
    capture: Option<u32>,
    limit: usize,
) -> Result<()> {
    find_collision(handle, address, owned, capture, limit)
        .await
        .map_err(|mismatch| mismatch.error)
}
async fn find_collision(
    handle: &Handle,
    address: Ipv4Addr,
    owned: Option<u32>,
    capture: Option<u32>,
    limit: usize,
) -> std::result::Result<(), Mismatch> {
    let addresses = get_addresses(handle, limit)
        .await
        .map_err(Mismatch::dump(ADDRESSES))?;
    if let Some(entry) = addresses.iter().find(|entry| Some(entry.header.index) != owned && entry.attributes.iter().any(|a| matches!(a, AddressAttribute::Address(IpAddr::V4(ip)) | AddressAttribute::Local(IpAddr::V4(ip)) if *ip == address))) {
        return Err(Mismatch::new(
            "anchor_collision",
            format!("{address} also on index {}", entry.header.index),
            ADDRESSES,
        ));
    }
    if let Some(route) = get_routes(handle, limit)
        .await
        .map_err(Mismatch::dump(ROUTES))?
        .iter()
        .filter(|route| capture != Some(super::route_table(route)))
        .find(|route| route_conflicts(address, owned, route))
    {
        return Err(Mismatch::new(
            "anchor_collision",
            describe_route(route),
            ROUTES,
        ));
    }
    Ok(())
}
pub(super) async fn verify(
    handle: &Handle,
    record: &Record,
    capture: Option<u32>,
    limit: usize,
) -> Result<()> {
    inspect(handle, record, capture, limit)
        .await
        .map_err(|mismatch| mismatch.error)
}
async fn inspect(
    handle: &Handle,
    record: &Record,
    capture: Option<u32>,
    limit: usize,
) -> std::result::Result<(), Mismatch> {
    // The journal that holds this record carries the namespace cookie; the
    // anchor itself is proven resident by its link and alias below.
    let current = super::namespace::Identity::current().map_err(|error| Mismatch {
        actual: format!("namespace unreadable: {error}"),
        error,
        check: "anchor_namespace",
        source: "/proc/self/ns/net",
    })?;
    if !current.is(
        &record.identity.boot,
        record.identity.namespace_device,
        record.identity.namespace_inode,
        None,
    ) {
        return Err(Mismatch::new(
            "anchor_namespace",
            format!("netns {}:{}", current.device, current.inode),
            "/proc/self/ns/net",
        ));
    }
    let link = find_owned_link(record, limit).await?;
    if !link_matches(record, &link) {
        return Err(Mismatch::new("anchor_link", describe_link(&link), LINKS));
    }
    find_addresses(handle, record, limit).await?;
    find_collision(
        handle,
        record.identity.address,
        Some(record.identity.index),
        capture,
        limit,
    )
    .await
}

async fn owned_link(record: &Record, limit: usize) -> Result<LinkMessage> {
    find_owned_link(record, limit)
        .await
        .map_err(|mismatch| mismatch.error)
}
async fn find_owned_link(
    record: &Record,
    limit: usize,
) -> std::result::Result<LinkMessage, Mismatch> {
    // Only links with its index, name or alias are decoded (G16): the
    // proof's cost does not grow with unrelated interfaces.
    let links = super::rtnl::links_named(
        record.identity.index,
        &record.identity.name,
        &record.alias,
        limit,
    )
    .await
    .map_err(Mismatch::dump(LINKS))?;
    let matches: Vec<_> = links
        .iter()
        .filter(|link| {
            link.header.index == record.identity.index
                || link.attributes.iter().any(|a| {
                    matches!(a, LinkAttribute::IfName(v) if v == &record.identity.name)
                        || matches!(a, LinkAttribute::IfAlias(v) if v == &record.alias)
                })
        })
        .collect();
    match matches.as_slice() {
        [link] if device_matches(record, link) => Ok((*link).clone()),
        [link] => Err(Mismatch::new("anchor_link", describe_link(link), LINKS)),
        links => Err(Mismatch::new(
            "anchor_link",
            format!(
                "{} links with its index, name or alias{}",
                links.len(),
                links
                    .iter()
                    .map(|link| format!("; {}", describe_link(link)))
                    .collect::<String>()
            ),
            LINKS,
        )),
    }
}

async fn verify_addresses(handle: &Handle, record: &Record, limit: usize) -> Result<()> {
    find_addresses(handle, record, limit)
        .await
        .map_err(|mismatch| mismatch.error)
}
async fn find_addresses(
    handle: &Handle,
    record: &Record,
    limit: usize,
) -> std::result::Result<(), Mismatch> {
    let addresses = get_addresses(handle, limit)
        .await
        .map_err(Mismatch::dump(ADDRESSES))?;
    let owned: Vec<_> = addresses
        .iter()
        .filter(|address| address.header.index == record.identity.index)
        .collect();
    match owned.as_slice() {
        [address] if address_matches(record, address) => Ok(()),
        owned => Err(Mismatch::new(
            "anchor_address",
            format!(
                "{} addresses on index {}{}",
                owned.len(),
                record.identity.index,
                owned
                    .iter()
                    .map(|address| format!(
                        "; {:?}/{} scope {:?}",
                        address.attributes.iter().find_map(|a| match a {
                            AddressAttribute::Local(ip) | AddressAttribute::Address(ip) =>
                                Some(*ip),
                            _ => None,
                        }),
                        address.header.prefix_len,
                        address.header.scope
                    ))
                    .collect::<String>()
            ),
            ADDRESSES,
        )),
    }
}
